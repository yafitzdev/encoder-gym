use std::{collections::BTreeMap, path::Path};

use analysis_core::{contract::DiagnosticContract, ports::AnalysisStore};
use anyhow::{Context, bail};
use dataset_core::ports::SnapshotStore;
use evaluation_core::ports::EvaluationStore;
use generation_core::ports::{DatasetStore, PlanStore, RowStore};
use optimization_core::{
    application::approved_proposal_to_plan,
    domain::{OptimizationProposal, ProposalApplication},
    evidence::OptimizationEvidence,
    planning::{
        create_constrained_proposal_with_training, create_proposal, proposal_to_plan,
        rebase_constrained_proposal,
    },
    ports::{OptimizationProposalQuery, OptimizationStore, ProposalReviewQuery},
    protocol::{OptimizationProtocol, RecommendationKind},
    reviews::{ProposalReviewRecord, ProposalReviewState},
    scenarios::create_default_scenario_group,
    training_candidates::{TrainingConfigurationChoice, TrainingConfigurationSpace},
};
use synthetic_data_sqlite::SqliteStore;
use training_core::ports::{EncoderRegistry, TrainingStore};

use crate::cli::{
    ExportFormat, OptimizationRecommendationKindArg, OptimizeCommand, PageArgs,
    ProposalReviewStateArg,
};

pub async fn execute(command: OptimizeCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        OptimizeCommand::Preview {
            analysis_report_id,
            protocol,
            training_space,
        } => {
            let protocol = read_protocol(&protocol)?;
            let training_space = read_optional_training_space(training_space.as_deref())?;
            let proposal = decision_proposal(
                store,
                analysis_report_id,
                &protocol,
                training_space.as_ref(),
            )
            .await?;
            print_json(&serde_json::json!({"persisted": false, "proposal": proposal}))?;
        }
        OptimizeCommand::Propose {
            analysis_report_id,
            protocol,
            budget,
            minimum_support,
            training_space,
        } => {
            let proposal = if let Some(protocol_path) = protocol {
                let protocol = read_protocol(&protocol_path)?;
                let training_space = read_optional_training_space(training_space.as_deref())?;
                decision_proposal(
                    store,
                    analysis_report_id,
                    &protocol,
                    training_space.as_ref(),
                )
                .await?
            } else {
                legacy_proposal(
                    store,
                    analysis_report_id,
                    budget.context("--budget is required without --protocol")?,
                    minimum_support,
                )
                .await?
            };
            store.create_optimization_proposal(&proposal).await?;
            print_json(&proposal)?;
        }
        OptimizeCommand::Scenarios {
            analysis_report_id,
            protocol,
        } => {
            let mut protocol = read_protocol(&protocol)?;
            protocol.recommendation_kinds = vec![RecommendationKind::DataGeneration];
            protocol.training_candidates = None;
            let protocol = protocol.normalize()?;
            let context = load_context(store, analysis_report_id).await?;
            let evidence = context.evidence(&protocol, None)?;
            let group = create_default_scenario_group(&evidence, &protocol)?;
            store.create_scenario_group(&group).await?;
            print_json(&group)?;
        }
        OptimizeCommand::ScenarioShow { id } => {
            let group = store
                .get_scenario_group(id)
                .await?
                .with_context(|| format!("optimization scenario group not found: {id}"))?;
            print_json(&group)?;
        }
        OptimizeCommand::ScenarioMaterialize {
            group_id,
            scenario_id,
        } => {
            let group = store
                .get_scenario_group(group_id)
                .await?
                .with_context(|| format!("optimization scenario group not found: {group_id}"))?;
            let scenario = group
                .scenarios
                .iter()
                .find(|scenario| scenario.id == scenario_id)
                .with_context(|| {
                    format!("scenario not found in group {group_id}: {scenario_id}")
                })?;
            let proposal = &scenario.proposal;
            let already_materialized =
                if let Some(existing) = store.get_optimization_proposal(proposal.id).await? {
                    if existing != *proposal {
                        bail!("proposal identity collision while materializing scenario");
                    }
                    true
                } else {
                    store.create_optimization_proposal(proposal).await?;
                    false
                };
            print_json(&serde_json::json!({
                "group_id": group_id,
                "scenario_id": scenario_id,
                "proposal": proposal,
                "already_materialized": already_materialized,
            }))?;
        }
        OptimizeCommand::List {
            analysis_report_id,
            dataset_id,
            page,
        } => {
            let proposals = store
                .query_optimization_proposals(OptimizationProposalQuery {
                    analysis_report_id,
                    dataset_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&proposals, proposals.len(), page)?;
        }
        OptimizeCommand::Show { id } => {
            let proposal = require_proposal(store, id).await?;
            print_json(&serde_json::json!({
                "proposal": proposal,
                "reviews": store.query_proposal_reviews(ProposalReviewQuery {
                    proposal_id: Some(id), state: None, limit: 10_000, offset: 0,
                }).await?,
                "application": store.get_proposal_application(id).await?,
            }))?;
        }
        OptimizeCommand::Recommendations {
            id,
            kind,
            label,
            dimension,
            eligible,
            minimum_score,
            page,
        } => {
            if minimum_score.is_some_and(|score| !score.is_finite()) {
                bail!("--minimum-score must be finite");
            }
            let proposal = require_proposal(store, id).await?;
            let dimension = dimension
                .as_deref()
                .map(parse_dimension_filter)
                .transpose()?;
            let requested_kind = kind.map(recommendation_kind);
            let data = proposal
                .normalized_recommendations
                .iter()
                .filter(|recommendation| {
                    requested_kind.is_none_or(|kind| recommendation.kind == kind)
                        && label
                            .as_ref()
                            .is_none_or(|label| &recommendation.cell.label == label)
                        && dimension.as_ref().is_none_or(|(name, value)| {
                            recommendation.cell.dimensions.get(name) == Some(value)
                        })
                        && (!eligible || recommendation.eligibility.eligible)
                        && minimum_score
                            .is_none_or(|minimum| recommendation.score.final_score >= minimum)
                })
                .collect::<Vec<_>>();
            let training = proposal
                .training_candidate_set
                .as_ref()
                .map(|set| set.candidates.as_slice())
                .unwrap_or_default()
                .iter()
                .filter(|_| {
                    requested_kind
                        .is_none_or(|kind| kind == RecommendationKind::TrainingConfiguration)
                        && label.is_none()
                        && dimension.is_none()
                        && minimum_score.is_none()
                })
                .collect::<Vec<_>>();
            let review_only = proposal
                .review_only_recommendations
                .iter()
                .filter(|recommendation| {
                    requested_kind.is_none_or(|kind| kind == RecommendationKind::ReviewOnly)
                        && label
                            .as_ref()
                            .is_none_or(|label| &recommendation.cell.label == label)
                        && dimension.as_ref().is_none_or(|(name, value)| {
                            recommendation.cell.dimensions.get(name) == Some(value)
                        })
                        && minimum_score
                            .is_none_or(|minimum| recommendation.score.final_score >= minimum)
                })
                .collect::<Vec<_>>();
            let data_total = data.len();
            let training_total = training.len();
            let review_only_total = review_only.len();
            let data = page_slice(&data, page);
            let training = page_slice(&training, page);
            let review_only = page_slice(&review_only, page);
            print_json(&serde_json::json!({
                "proposal_id": id,
                "data_generation": data,
                "training_configuration": training,
                "review_only": review_only,
                "page": {
                    "limit": page.limit,
                    "offset": page.offset,
                    "data_generation_total": data_total,
                    "training_configuration_total": training_total,
                    "review_only_total": review_only_total,
                },
            }))?;
        }
        OptimizeCommand::Explain {
            id,
            recommendation_id,
        } => {
            let proposal = require_proposal(store, id).await?;
            if let Some(recommendation) = proposal
                .normalized_recommendations
                .iter()
                .find(|recommendation| recommendation.id == recommendation_id)
            {
                print_json(&serde_json::json!({
                    "kind": "data_generation",
                    "recommendation": recommendation,
                    "decision": proposal.decision_cells.iter().find(|decision| {
                        decision.finding_key == recommendation.finding_key
                    }),
                }))?;
            } else if let Some(candidate) =
                proposal.training_candidate_set.as_ref().and_then(|set| {
                    set.candidates
                        .iter()
                        .find(|candidate| candidate.id == recommendation_id)
                })
            {
                print_json(&serde_json::json!({
                    "kind": "training_configuration",
                    "candidate": candidate,
                }))?;
            } else if let Some(recommendation) = proposal
                .review_only_recommendations
                .iter()
                .find(|recommendation| recommendation.id == recommendation_id)
            {
                print_json(&serde_json::json!({
                    "kind": "review_only",
                    "recommendation": recommendation,
                }))?;
            } else {
                bail!("recommendation not found in proposal {id}: {recommendation_id}");
            }
        }
        OptimizeCommand::Review {
            id,
            state,
            selected_recommendation_ids,
            note,
            superseding_proposal_id,
            campaign_id,
        } => {
            let proposal = require_proposal(store, id).await?;
            let review = ProposalReviewRecord::new(
                &proposal,
                review_state(state),
                selected_recommendation_ids,
                note,
                superseding_proposal_id,
                campaign_id,
            )?;
            store.append_proposal_review(&review).await?;
            print_json(&review)?;
        }
        OptimizeCommand::Apply { id, approval_id } => {
            let proposal = require_proposal(store, id).await?;
            if let Some(application) = store.get_proposal_application(id).await? {
                let plan = store
                    .get_plan(application.generation_plan_id)
                    .await?
                    .context("applied optimization plan is missing")?;
                print_json(&serde_json::json!({
                    "proposal_id": proposal.id,
                    "generation_plan": plan,
                    "application": application,
                    "already_applied": true,
                }))?;
                return Ok(());
            }
            let approval = require_review(store, id, approval_id).await?;
            let dataset = store
                .get_dataset(proposal.dataset_id)
                .await?
                .with_context(|| format!("dataset not found: {}", proposal.dataset_id))?;
            let accepted = accepted_coverage(store, dataset.id).await?;
            let (plan, application) =
                approved_proposal_to_plan(&proposal, &approval, &dataset, &accepted)?;
            let application = store.apply_proposal_plan(&plan, &application).await?;
            print_json(&serde_json::json!({
                "proposal_id": proposal.id,
                "generation_plan": plan,
                "application": application,
                "already_applied": false,
                "generation_started": false,
            }))?;
        }
        OptimizeCommand::LegacyApply { id } => legacy_apply(store, id).await?,
        OptimizeCommand::Rebase { id } => {
            let proposal = require_proposal(store, id).await?;
            let protocol = proposal
                .protocol
                .as_ref()
                .context("legacy proposals cannot be rebased")?;
            let context = load_context(store, proposal.analysis_report_id).await?;
            let training_fingerprint = proposal
                .training_candidate_set
                .as_ref()
                .map(|set| set.configuration_space.fingerprint.clone());
            let evidence = context.evidence(protocol, training_fingerprint)?;
            let rebased = rebase_constrained_proposal(&proposal, &evidence)?;
            store.create_optimization_proposal(&rebased).await?;
            print_json(&rebased)?;
        }
        OptimizeCommand::TrainingSpace {
            training_run_id,
            checkpoint_id,
            choices,
            output,
        } => {
            let run = store
                .get_training_run(training_run_id)
                .await?
                .with_context(|| format!("training run not found: {training_run_id}"))?;
            let checkpoint = store
                .get_checkpoint(checkpoint_id)
                .await?
                .with_context(|| format!("training checkpoint not found: {checkpoint_id}"))?;
            let snapshot = store
                .get_snapshot(run.snapshot_id)
                .await?
                .with_context(|| format!("training snapshot not found: {}", run.snapshot_id))?;
            let encoder = if let Some(id) = run.base_model_id {
                Some(
                    store
                        .get_encoder(id)
                        .await?
                        .with_context(|| format!("registered encoder not found: {id}"))?,
                )
            } else {
                None
            };
            let choices = read_training_choices(&choices)?;
            let space = TrainingConfigurationSpace::new(
                &run,
                &checkpoint,
                snapshot.fingerprint,
                encoder.as_ref(),
                choices,
            )?;
            std::fs::write(&output, serde_json::to_vec_pretty(&space)?)
                .with_context(|| format!("could not write {}", output.display()))?;
            print_json(&serde_json::json!({
                "created": true,
                "artifact": "training_configuration_space",
                "fingerprint": space.fingerprint,
                "choices": space.choices.len(),
                "output": output,
                "training_started": false,
            }))?;
        }
        OptimizeCommand::TrainingCandidates { id, format, output } => {
            let proposal = require_proposal(store, id).await?;
            if let Some(output) = output {
                export_training_candidates(&proposal, format, &output)?;
                print_json(&serde_json::json!({
                    "exported": true,
                    "artifact": "optimization_training_candidates",
                    "rows": proposal.training_candidate_set.as_ref()
                        .map_or(0, |set| set.candidates.len()),
                    "output": output,
                    "training_started": false,
                }))?;
                return Ok(());
            }
            print_json(&serde_json::json!({
                "proposal_id": id,
                "training_candidate_set": proposal.training_candidate_set,
                "training_started": false,
            }))?;
        }
        OptimizeCommand::ExportRecommendations { id, format, output } => {
            let proposal = require_proposal(store, id).await?;
            export_recommendations(&proposal, format, &output)?;
            print_json(&serde_json::json!({
                "exported": true,
                "artifact": "optimization_recommendations",
                "rows": proposal.normalized_recommendations.len(),
                "output": output,
            }))?;
        }
        OptimizeCommand::ExportSummary { id, format, output } => {
            let proposal = require_proposal(store, id).await?;
            export_proposal_summary(&proposal, format, &output)?;
            print_json(&serde_json::json!({
                "exported": true,
                "artifact": "optimization_proposal_summary",
                "rows": 1,
                "output": output,
            }))?;
        }
        OptimizeCommand::Export { id, output } => {
            let proposal = require_proposal(store, id).await?;
            std::fs::write(&output, serde_json::to_vec_pretty(&proposal)?)
                .with_context(|| format!("could not write {}", output.display()))?;
            crate::presentation::print(&serde_json::json!({
                "exported": true,
                "artifact": "optimization_proposal",
                "output": output,
            }))?;
        }
    }
    Ok(())
}

async fn decision_proposal(
    store: &SqliteStore,
    analysis_report_id: uuid::Uuid,
    protocol: &OptimizationProtocol,
    training_space: Option<&TrainingConfigurationSpace>,
) -> anyhow::Result<OptimizationProposal> {
    let context = load_context(store, analysis_report_id).await?;
    let training_fingerprint = training_space.map(|space| space.fingerprint.clone());
    let evidence = context.evidence(protocol, training_fingerprint)?;
    Ok(create_constrained_proposal_with_training(
        &evidence,
        protocol,
        training_space,
    )?)
}

pub(crate) async fn propose_workflow(
    store: &SqliteStore,
    analysis_report_id: uuid::Uuid,
    protocol: &OptimizationProtocol,
) -> anyhow::Result<OptimizationProposal> {
    let protocol = protocol.clone().normalize()?;
    let protocol_fingerprint = protocol.fingerprint()?;
    if let Some(existing) = store
        .query_optimization_proposals(OptimizationProposalQuery {
            analysis_report_id: Some(analysis_report_id),
            dataset_id: None,
            limit: 10_000,
            offset: 0,
        })
        .await?
        .into_iter()
        .find(|proposal| proposal.protocol_fingerprint == protocol_fingerprint)
    {
        return Ok(existing);
    }
    let proposal = decision_proposal(store, analysis_report_id, &protocol, None).await?;
    store.create_optimization_proposal(&proposal).await?;
    Ok(proposal)
}

async fn legacy_proposal(
    store: &SqliteStore,
    analysis_report_id: uuid::Uuid,
    budget: u32,
    minimum_support: u64,
) -> anyhow::Result<OptimizationProposal> {
    let context = load_context(store, analysis_report_id).await?;
    Ok(create_proposal(
        &context.report,
        &context.dataset,
        &context.accepted,
        budget,
        minimum_support,
    )?)
}

async fn legacy_apply(store: &SqliteStore, id: uuid::Uuid) -> anyhow::Result<()> {
    let proposal = require_proposal(store, id).await?;
    if proposal.source_identity.is_some() || !proposal.normalized_recommendations.is_empty() {
        bail!(
            "decision-grade proposals require `optimize apply --approval-id`; legacy-apply only accepts historical proposals"
        );
    }
    if let Some(application) = store.get_proposal_application(id).await? {
        let plan = store
            .get_plan(application.generation_plan_id)
            .await?
            .context("applied optimization plan is missing")?;
        print_json(&serde_json::json!({
            "proposal_id": proposal.id,
            "generation_plan": plan,
            "application": application,
            "already_applied": true,
            "legacy_compatibility_path": true,
        }))?;
        return Ok(());
    }
    let dataset = store
        .get_dataset(proposal.dataset_id)
        .await?
        .with_context(|| format!("dataset not found: {}", proposal.dataset_id))?;
    let plan = proposal_to_plan(&proposal, &dataset)?;
    let application = ProposalApplication {
        proposal_id: proposal.id,
        generation_plan_id: plan.id,
        approval_review_id: None,
        approval_fingerprint: None,
        selected_recommendation_ids: Vec::new(),
        verified_coverage_fingerprint: None,
        applied_at: chrono::Utc::now(),
    };
    let application = store.apply_proposal_plan(&plan, &application).await?;
    print_json(&serde_json::json!({
        "proposal_id": proposal.id,
        "generation_plan": plan,
        "application": application,
        "already_applied": false,
        "legacy_compatibility_path": true,
    }))?;
    Ok(())
}

struct OptimizationContext {
    report: analysis_core::domain::AnalysisReport,
    dataset: generation_core::domain::DatasetDefinition,
    snapshot_id: uuid::Uuid,
    snapshot_fingerprint: String,
    accepted: BTreeMap<String, u32>,
}

impl OptimizationContext {
    fn evidence(
        &self,
        protocol: &OptimizationProtocol,
        training_configuration_space_fingerprint: Option<String>,
    ) -> anyhow::Result<OptimizationEvidence> {
        let diagnostics = DiagnosticContract::try_from(&self.report)?;
        Ok(OptimizationEvidence::new(
            diagnostics,
            &self.dataset,
            self.snapshot_id,
            self.snapshot_fingerprint.clone(),
            &self.accepted,
            protocol,
            training_configuration_space_fingerprint,
            true,
        )?)
    }
}

async fn load_context(
    store: &SqliteStore,
    analysis_report_id: uuid::Uuid,
) -> anyhow::Result<OptimizationContext> {
    let report = store
        .get_analysis_report(analysis_report_id)
        .await?
        .with_context(|| format!("analysis report not found: {analysis_report_id}"))?;
    let evaluation_run = store
        .get_evaluation_run(report.evaluation_run_id)
        .await?
        .with_context(|| format!("evaluation run not found: {}", report.evaluation_run_id))?;
    let snapshot = store
        .get_snapshot(evaluation_run.snapshot_id)
        .await?
        .with_context(|| format!("snapshot not found: {}", evaluation_run.snapshot_id))?;
    let dataset = store
        .get_dataset(snapshot.source_dataset_id)
        .await?
        .with_context(|| format!("source dataset not found: {}", snapshot.source_dataset_id))?;
    let accepted = accepted_coverage(store, dataset.id).await?;
    Ok(OptimizationContext {
        report,
        dataset,
        snapshot_id: snapshot.id,
        snapshot_fingerprint: snapshot.fingerprint,
        accepted,
    })
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

async fn require_proposal(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<OptimizationProposal> {
    store
        .get_optimization_proposal(id)
        .await?
        .with_context(|| format!("optimization proposal not found: {id}"))
}

async fn require_review(
    store: &SqliteStore,
    proposal_id: uuid::Uuid,
    review_id: uuid::Uuid,
) -> anyhow::Result<ProposalReviewRecord> {
    store
        .query_proposal_reviews(ProposalReviewQuery {
            proposal_id: Some(proposal_id),
            state: None,
            limit: 10_000,
            offset: 0,
        })
        .await?
        .into_iter()
        .find(|review| review.id == review_id)
        .with_context(|| format!("proposal review not found: {review_id}"))
}

fn read_protocol(path: &Path) -> anyhow::Result<OptimizationProtocol> {
    let protocol: OptimizationProtocol = read_json_or_toml(path)?;
    Ok(protocol.normalize()?)
}

fn read_optional_training_space(
    path: Option<&Path>,
) -> anyhow::Result<Option<TrainingConfigurationSpace>> {
    path.map(|path| {
        let space: TrainingConfigurationSpace = read_json_or_toml(path)?;
        space.validate()?;
        Ok(space)
    })
    .transpose()
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainingChoicesFile {
    choices: Vec<TrainingConfigurationChoice>,
}

fn read_training_choices(path: &Path) -> anyhow::Result<Vec<TrainingConfigurationChoice>> {
    let file: TrainingChoicesFile = read_json_or_toml(path)?;
    if file.choices.is_empty() {
        bail!("training choices must contain at least one explicit candidate");
    }
    Ok(file.choices)
}

fn read_json_or_toml<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    if path.extension().and_then(|value| value.to_str()) == Some("json") {
        serde_json::from_str(&contents)
            .with_context(|| format!("invalid JSON in {}", path.display()))
    } else {
        toml::from_str(&contents).with_context(|| format!("invalid TOML in {}", path.display()))
    }
}

fn parse_dimension_filter(value: &str) -> anyhow::Result<(String, String)> {
    let (name, value) = value
        .split_once('=')
        .context("--dimension must use name=value")?;
    if name.trim().is_empty() || value.trim().is_empty() {
        bail!("--dimension must use non-empty name=value");
    }
    Ok((name.trim().into(), value.trim().into()))
}

fn page_slice<T>(items: &[T], page: PageArgs) -> &[T] {
    let start = usize::try_from(page.offset)
        .unwrap_or(usize::MAX)
        .min(items.len());
    let end = start
        .saturating_add(usize::try_from(page.limit).unwrap_or(usize::MAX))
        .min(items.len());
    &items[start..end]
}

const fn recommendation_kind(value: OptimizationRecommendationKindArg) -> RecommendationKind {
    match value {
        OptimizationRecommendationKindArg::DataGeneration => RecommendationKind::DataGeneration,
        OptimizationRecommendationKindArg::TrainingConfiguration => {
            RecommendationKind::TrainingConfiguration
        }
        OptimizationRecommendationKindArg::ReviewOnly => RecommendationKind::ReviewOnly,
    }
}

const fn review_state(value: ProposalReviewStateArg) -> ProposalReviewState {
    match value {
        ProposalReviewStateArg::Open => ProposalReviewState::Open,
        ProposalReviewStateArg::ApprovedForPlanCreation => {
            ProposalReviewState::ApprovedForPlanCreation
        }
        ProposalReviewStateArg::Rejected => ProposalReviewState::Rejected,
        ProposalReviewStateArg::Superseded => ProposalReviewState::Superseded,
        ProposalReviewStateArg::PartiallyAccepted => ProposalReviewState::PartiallyAccepted,
        ProposalReviewStateArg::AcceptedTrainingExperimentCandidate => {
            ProposalReviewState::AcceptedTrainingExperimentCandidate
        }
        ProposalReviewStateArg::AcceptedReviewOnlyCandidates => {
            ProposalReviewState::AcceptedReviewOnlyCandidates
        }
        ProposalReviewStateArg::CompletedAwaitingOutcomeAssessment => {
            ProposalReviewState::CompletedAwaitingOutcomeAssessment
        }
    }
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    crate::presentation::print(value)
}

fn export_recommendations(
    proposal: &OptimizationProposal,
    format: ExportFormat,
    output: &Path,
) -> anyhow::Result<()> {
    match format {
        ExportFormat::Jsonl => {
            let mut contents = String::new();
            for recommendation in &proposal.normalized_recommendations {
                contents.push_str(&serde_json::to_string(recommendation)?);
                contents.push('\n');
            }
            std::fs::write(output, contents)
                .with_context(|| format!("could not write {}", output.display()))?;
        }
        ExportFormat::Csv => {
            let mut writer = csv::Writer::from_path(output)
                .with_context(|| format!("could not create {}", output.display()))?;
            writer.write_record([
                "proposal_id",
                "recommendation_id",
                "kind",
                "cell_key",
                "label",
                "dimensions_json",
                "eligible",
                "score",
                "current_accepted",
                "additional_count",
                "proposed_target",
                "finding_key",
                "finding_fingerprint",
                "evidence_fingerprint",
                "rationale",
            ])?;
            for recommendation in &proposal.normalized_recommendations {
                writer.write_record([
                    proposal.id.to_string(),
                    recommendation.id.clone(),
                    recommendation_kind_name(recommendation.kind).into(),
                    recommendation.cell.key(),
                    recommendation.cell.label.clone(),
                    serde_json::to_string(&recommendation.cell.dimensions)?,
                    recommendation.eligibility.eligible.to_string(),
                    recommendation.score.final_score.to_string(),
                    recommendation.current_accepted.to_string(),
                    recommendation.additional_count.to_string(),
                    recommendation.proposed_target.to_string(),
                    recommendation.finding_key.clone(),
                    recommendation.finding_fingerprint.clone(),
                    recommendation.evidence_fingerprint.clone(),
                    recommendation.rationale.clone(),
                ])?;
            }
            writer.flush()?;
        }
    }
    Ok(())
}

fn export_training_candidates(
    proposal: &OptimizationProposal,
    format: ExportFormat,
    output: &Path,
) -> anyhow::Result<()> {
    let candidates = proposal
        .training_candidate_set
        .as_ref()
        .map(|set| set.candidates.as_slice())
        .unwrap_or_default();
    match format {
        ExportFormat::Jsonl => {
            let mut contents = String::new();
            for candidate in candidates {
                contents.push_str(&serde_json::to_string(candidate)?);
                contents.push('\n');
            }
            std::fs::write(output, contents)
                .with_context(|| format!("could not write {}", output.display()))?;
        }
        ExportFormat::Csv => {
            let mut writer = csv::Writer::from_path(output)
                .with_context(|| format!("could not create {}", output.display()))?;
            writer.write_record([
                "proposal_id",
                "candidate_id",
                "baseline_training_run_id",
                "baseline_checkpoint_id",
                "snapshot_id",
                "configuration_space_fingerprint",
                "changed_fields_json",
                "configuration_json",
                "transformer_configuration_json",
                "rationale",
                "cautions_json",
            ])?;
            for candidate in candidates {
                writer.write_record([
                    proposal.id.to_string(),
                    candidate.id.clone(),
                    candidate.baseline.training_run_id.to_string(),
                    candidate.baseline.checkpoint_id.to_string(),
                    candidate.baseline.snapshot_id.to_string(),
                    candidate.configuration_space_fingerprint.clone(),
                    serde_json::to_string(&candidate.differences)?,
                    serde_json::to_string(&candidate.configuration)?,
                    serde_json::to_string(&candidate.transformer_configuration)?,
                    candidate.rationale.clone(),
                    serde_json::to_string(&candidate.cautions)?,
                ])?;
            }
            writer.flush()?;
        }
    }
    Ok(())
}

fn export_proposal_summary(
    proposal: &OptimizationProposal,
    format: ExportFormat,
    output: &Path,
) -> anyhow::Result<()> {
    let allocation = proposal.allocation.as_ref();
    let summary = serde_json::json!({
        "proposal_id": proposal.id,
        "analysis_report_id": proposal.analysis_report_id,
        "dataset_id": proposal.dataset_id,
        "protocol_fingerprint": proposal.protocol_fingerprint,
        "evidence_fingerprint": proposal.evidence_fingerprint,
        "requested_budget": proposal.additional_example_budget,
        "allocated_budget": allocation.map_or(proposal.allocated_count(), |value| u64::from(value.allocated_budget)),
        "unallocated_budget": allocation.map_or(0, |value| value.unallocated_budget),
        "data_recommendations": proposal.normalized_recommendations.len(),
        "training_candidates": proposal.training_candidate_set.as_ref().map_or(0, |value| value.candidates.len()),
        "review_only_recommendations": proposal.review_only_recommendations.len(),
        "proposal_fingerprint": proposal.fingerprint,
        "created_at": proposal.created_at,
    });
    match format {
        ExportFormat::Jsonl => {
            let contents = format!("{}\n", serde_json::to_string(&summary)?);
            std::fs::write(output, contents)
                .with_context(|| format!("could not write {}", output.display()))?;
        }
        ExportFormat::Csv => {
            let mut writer = csv::Writer::from_path(output)
                .with_context(|| format!("could not create {}", output.display()))?;
            writer.write_record([
                "proposal_id",
                "analysis_report_id",
                "dataset_id",
                "protocol_fingerprint",
                "evidence_fingerprint",
                "requested_budget",
                "allocated_budget",
                "unallocated_budget",
                "data_recommendations",
                "training_candidates",
                "review_only_recommendations",
                "proposal_fingerprint",
                "created_at",
            ])?;
            writer.write_record([
                proposal.id.to_string(),
                proposal.analysis_report_id.to_string(),
                proposal.dataset_id.to_string(),
                proposal.protocol_fingerprint.clone(),
                proposal.evidence_fingerprint.clone(),
                proposal.additional_example_budget.to_string(),
                allocation
                    .map_or(proposal.allocated_count(), |value| {
                        u64::from(value.allocated_budget)
                    })
                    .to_string(),
                allocation
                    .map_or(0, |value| value.unallocated_budget)
                    .to_string(),
                proposal.normalized_recommendations.len().to_string(),
                proposal
                    .training_candidate_set
                    .as_ref()
                    .map_or(0, |value| value.candidates.len())
                    .to_string(),
                proposal.review_only_recommendations.len().to_string(),
                proposal.fingerprint.clone(),
                proposal.created_at.to_rfc3339(),
            ])?;
            writer.flush()?;
        }
    }
    Ok(())
}

const fn recommendation_kind_name(kind: RecommendationKind) -> &'static str {
    match kind {
        RecommendationKind::DataGeneration => "data_generation",
        RecommendationKind::TrainingConfiguration => "training_configuration",
        RecommendationKind::ReviewOnly => "review_only",
    }
}
