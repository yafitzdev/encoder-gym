use std::collections::{BTreeMap, BTreeSet};

use analysis_core::domain::AnalysisReport;
use chrono::{DateTime, Utc};
use dataset_core::domain::{DatasetSnapshot, SnapshotMember, SourceProvenance};
use evaluation_core::domain::{
    ConfidenceInterval, EvaluationComparisonReport, EvaluationRun, EvaluationRunState,
};
use generation_core::{domain::GenerationPlan, jobs::GenerationJob};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use training_core::domain::{TrainingCheckpoint, TrainingRun};
use uuid::Uuid;

use crate::{
    domain::{OptimizationProposal, ProposalApplication},
    planning::verify_constrained_proposal,
    reviews::{ProposalReviewRecord, ProposalReviewState},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OptimizationCampaign {
    pub id: Uuid,
    pub proposal_id: Uuid,
    pub proposal_fingerprint: String,
    pub approval_review_id: Uuid,
    pub approval_fingerprint: String,
    pub selected_recommendation_ids: Vec<String>,
    pub baseline_analysis_report_id: Uuid,
    pub baseline_analysis_fingerprint: String,
    pub baseline_evaluation_run_id: Uuid,
    pub baseline_comparison_id: Option<Uuid>,
    pub dataset_id: Uuid,
    pub source_snapshot_id: Uuid,
    pub source_snapshot_fingerprint: String,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CampaignArtifactKind {
    GenerationPlan,
    GenerationJob,
    DatasetSnapshot,
    TrainingRun,
    TrainingCheckpoint,
    CandidateEvaluation,
    EvaluationComparison,
    FollowUpAnalysis,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CampaignLinkDetails {
    GenerationPlan {
        application_approval_id: Uuid,
        dataset_id: Uuid,
    },
    GenerationJob {
        plan_id: Uuid,
        dataset_id: Uuid,
    },
    DatasetSnapshot {
        dataset_id: Uuid,
        linked_generation_job_ids: Vec<Uuid>,
    },
    TrainingRun {
        snapshot_id: Uuid,
        backend_name: String,
    },
    TrainingCheckpoint {
        training_run_id: Uuid,
    },
    CandidateEvaluation {
        checkpoint_id: Uuid,
        snapshot_id: Uuid,
        cohort_fingerprint: String,
        protocol_fingerprint: String,
    },
    EvaluationComparison {
        baseline_evaluation_run_id: Uuid,
        candidate_evaluation_run_id: Uuid,
        cohort_fingerprint: String,
        protocol_fingerprint: String,
    },
    FollowUpAnalysis {
        evaluation_run_id: Uuid,
        comparison_id: Option<Uuid>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignArtifactLink {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub artifact_kind: CampaignArtifactKind,
    pub artifact_id: Uuid,
    pub artifact_fingerprint: String,
    pub details: CampaignLinkDetails,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeAssessmentPolicy {
    pub minimum_accuracy_delta: f64,
    pub minimum_macro_f1_delta: f64,
    pub require_significance: bool,
}

impl Default for OutcomeAssessmentPolicy {
    fn default() -> Self {
        Self {
            minimum_accuracy_delta: 0.0,
            minimum_macro_f1_delta: 0.0,
            require_significance: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeClassification {
    Improved,
    Regressed,
    Mixed,
    Inconclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetCoverageOutcome {
    pub cell_key: String,
    pub baseline_accepted: u32,
    pub proposed_target: u32,
    pub observed_accepted: u32,
    pub realized: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CampaignOutcomeAssessment {
    pub id: Uuid,
    pub campaign_id: Uuid,
    pub comparison_id: Uuid,
    pub comparison_fingerprint: String,
    pub policy: OutcomeAssessmentPolicy,
    pub policy_fingerprint: String,
    pub accuracy_delta: f64,
    pub macro_f1_delta: f64,
    pub per_label_f1_delta: BTreeMap<String, f64>,
    pub accuracy_delta_interval: ConfidenceInterval,
    pub macro_f1_delta_interval: ConfidenceInterval,
    pub mcnemar_p_value: f64,
    pub mcnemar_significant: bool,
    pub fixed_errors: u64,
    pub regressed_errors: u64,
    pub persistent_errors: u64,
    pub target_coverage: Vec<TargetCoverageOutcome>,
    pub constraints_realized: bool,
    pub classification: OutcomeClassification,
    pub caution: String,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

impl OptimizationCampaign {
    pub fn validate(&self) -> Result<(), CampaignError> {
        validate_campaign(self)
    }

    pub fn validate_decision(
        &self,
        proposal: &OptimizationProposal,
        approval: &ProposalReviewRecord,
    ) -> Result<(), CampaignError> {
        self.validate()?;
        verify_constrained_proposal(proposal)?;
        approval.validate(proposal)?;
        let source = proposal
            .source_identity
            .as_ref()
            .ok_or(CampaignError::LegacyProposal)?;
        let selected = match approval.state {
            ProposalReviewState::ApprovedForPlanCreation => proposal
                .normalized_recommendations
                .iter()
                .map(|recommendation| recommendation.id.clone())
                .collect::<Vec<_>>(),
            ProposalReviewState::PartiallyAccepted
            | ProposalReviewState::AcceptedTrainingExperimentCandidate
            | ProposalReviewState::AcceptedReviewOnlyCandidates => {
                approval.selected_recommendation_ids.clone()
            }
            _ => return Err(CampaignError::ApprovalState),
        };
        if self.proposal_id != proposal.id
            || self.proposal_fingerprint != proposal.fingerprint
            || self.approval_review_id != approval.id
            || self.approval_fingerprint != approval.fingerprint
            || self.selected_recommendation_ids != selected
            || self.baseline_analysis_report_id != source.analysis_report_id
            || self.baseline_analysis_fingerprint != source.analysis_fingerprint
            || self.baseline_evaluation_run_id != source.evaluation_run_id
            || self.baseline_comparison_id != source.comparison_id
            || self.dataset_id != source.dataset_id
            || self.source_snapshot_id != source.snapshot_id
            || self.source_snapshot_fingerprint != source.snapshot_fingerprint
        {
            return Err(CampaignError::ProposalIdentity);
        }
        Ok(())
    }
}

impl CampaignArtifactLink {
    pub fn validate(&self, campaign: &OptimizationCampaign) -> Result<(), CampaignError> {
        campaign.validate()?;
        if self.campaign_id != campaign.id || reproduce_fingerprint(self)? != self.fingerprint {
            return Err(CampaignError::LinkFingerprint);
        }
        Ok(())
    }
}

impl CampaignOutcomeAssessment {
    pub fn validate(
        &self,
        campaign: &OptimizationCampaign,
        comparison: &EvaluationComparisonReport,
    ) -> Result<(), CampaignError> {
        campaign.validate()?;
        validate_policy(self.policy)?;
        if self.campaign_id != campaign.id
            || self.comparison_id != comparison.id
            || self.comparison_fingerprint != comparison.fingerprint
            || evaluation_core::comparison::comparison_fingerprint(comparison)?
                != comparison.fingerprint
            || artifact_core::fingerprint(&self.policy)? != self.policy_fingerprint
            || reproduce_fingerprint(self)? != self.fingerprint
            || classify_outcome(comparison, self.policy)? != self.classification
        {
            return Err(CampaignError::OutcomeFingerprint);
        }
        Ok(())
    }

    pub fn validate_reproduction(
        &self,
        campaign: &OptimizationCampaign,
        proposal: &OptimizationProposal,
        links: &[CampaignArtifactLink],
        comparison: &EvaluationComparisonReport,
        observed_accepted_by_cell: &BTreeMap<String, u32>,
    ) -> Result<(), CampaignError> {
        self.validate(campaign, comparison)?;
        let expected = assess_campaign_outcome(
            campaign,
            proposal,
            links,
            comparison,
            observed_accepted_by_cell,
            self.policy,
        )?;
        let mut actual = self.clone();
        actual.id = expected.id;
        actual.created_at = expected.created_at;
        actual.fingerprint = expected.fingerprint.clone();
        if actual != expected {
            return Err(CampaignError::OutcomeFingerprint);
        }
        Ok(())
    }
}

pub fn create_campaign(
    proposal: &OptimizationProposal,
    approval: &ProposalReviewRecord,
) -> Result<OptimizationCampaign, CampaignError> {
    verify_constrained_proposal(proposal)?;
    approval.validate(proposal)?;
    if !matches!(
        approval.state,
        ProposalReviewState::ApprovedForPlanCreation
            | ProposalReviewState::PartiallyAccepted
            | ProposalReviewState::AcceptedTrainingExperimentCandidate
            | ProposalReviewState::AcceptedReviewOnlyCandidates
    ) {
        return Err(CampaignError::ApprovalState);
    }
    let source = proposal
        .source_identity
        .as_ref()
        .ok_or(CampaignError::LegacyProposal)?;
    let selected_recommendation_ids = match approval.state {
        ProposalReviewState::ApprovedForPlanCreation => proposal
            .normalized_recommendations
            .iter()
            .map(|recommendation| recommendation.id.clone())
            .collect(),
        _ => approval.selected_recommendation_ids.clone(),
    };
    let created_at = Utc::now();
    let mut campaign = OptimizationCampaign {
        id: Uuid::new_v4(),
        proposal_id: proposal.id,
        proposal_fingerprint: proposal.fingerprint.clone(),
        approval_review_id: approval.id,
        approval_fingerprint: approval.fingerprint.clone(),
        selected_recommendation_ids,
        baseline_analysis_report_id: source.analysis_report_id,
        baseline_analysis_fingerprint: source.analysis_fingerprint.clone(),
        baseline_evaluation_run_id: source.evaluation_run_id,
        baseline_comparison_id: source.comparison_id,
        dataset_id: source.dataset_id,
        source_snapshot_id: source.snapshot_id,
        source_snapshot_fingerprint: source.snapshot_fingerprint.clone(),
        fingerprint: String::new(),
        created_at,
    };
    campaign.fingerprint = reproduce_fingerprint(&campaign)?;
    Ok(campaign)
}

pub fn link_generation_plan(
    campaign: &OptimizationCampaign,
    plan: &GenerationPlan,
    application: &ProposalApplication,
) -> Result<CampaignArtifactLink, CampaignError> {
    validate_campaign(campaign)?;
    if application.proposal_id != campaign.proposal_id
        || application.generation_plan_id != plan.id
        || application.approval_review_id != Some(campaign.approval_review_id)
        || application.approval_fingerprint.as_deref()
            != Some(campaign.approval_fingerprint.as_str())
        || plan.dataset_id != campaign.dataset_id
    {
        return Err(CampaignError::GenerationPlanCompatibility);
    }
    make_link(
        campaign,
        CampaignArtifactKind::GenerationPlan,
        plan.id,
        artifact_core::fingerprint(plan)?,
        CampaignLinkDetails::GenerationPlan {
            application_approval_id: campaign.approval_review_id,
            dataset_id: plan.dataset_id,
        },
    )
}

pub fn link_generation_job(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    job: &GenerationJob,
) -> Result<CampaignArtifactLink, CampaignError> {
    require_link(
        campaign,
        links,
        CampaignArtifactKind::GenerationPlan,
        job.plan_id,
    )?;
    if job.dataset_id != campaign.dataset_id {
        return Err(CampaignError::GenerationJobCompatibility);
    }
    make_link(
        campaign,
        CampaignArtifactKind::GenerationJob,
        job.id,
        artifact_core::fingerprint(job)?,
        CampaignLinkDetails::GenerationJob {
            plan_id: job.plan_id,
            dataset_id: job.dataset_id,
        },
    )
}

pub fn link_snapshot(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
) -> Result<CampaignArtifactLink, CampaignError> {
    if snapshot.source_dataset_id != campaign.dataset_id
        || members.len() as u64 != snapshot.member_count
        || members
            .iter()
            .any(|member| member.snapshot_id != snapshot.id)
    {
        return Err(CampaignError::SnapshotCompatibility);
    }
    let linked_jobs = links
        .iter()
        .filter(|link| link.artifact_kind == CampaignArtifactKind::GenerationJob)
        .map(|link| link.artifact_id)
        .collect::<BTreeSet<_>>();
    let snapshot_jobs = members
        .iter()
        .filter_map(|member| match member.source_provenance {
            SourceProvenance::Generated {
                generation_job_id, ..
            } => Some(generation_job_id),
            SourceProvenance::Imported { .. } => None,
        })
        .collect::<BTreeSet<_>>();
    let campaign_jobs = snapshot_jobs
        .intersection(&linked_jobs)
        .copied()
        .collect::<Vec<_>>();
    if campaign_jobs.is_empty() || snapshot.fingerprint.trim().is_empty() {
        return Err(CampaignError::SnapshotLineage);
    }
    make_link(
        campaign,
        CampaignArtifactKind::DatasetSnapshot,
        snapshot.id,
        snapshot.fingerprint.clone(),
        CampaignLinkDetails::DatasetSnapshot {
            dataset_id: snapshot.source_dataset_id,
            linked_generation_job_ids: campaign_jobs,
        },
    )
}

pub fn link_training_run(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    run: &TrainingRun,
) -> Result<CampaignArtifactLink, CampaignError> {
    require_link(
        campaign,
        links,
        CampaignArtifactKind::DatasetSnapshot,
        run.snapshot_id,
    )?;
    make_link(
        campaign,
        CampaignArtifactKind::TrainingRun,
        run.id,
        artifact_core::fingerprint(run)?,
        CampaignLinkDetails::TrainingRun {
            snapshot_id: run.snapshot_id,
            backend_name: run.backend_name.clone(),
        },
    )
}

pub fn link_checkpoint(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    checkpoint: &TrainingCheckpoint,
) -> Result<CampaignArtifactLink, CampaignError> {
    require_link(
        campaign,
        links,
        CampaignArtifactKind::TrainingRun,
        checkpoint.run_id,
    )?;
    make_link(
        campaign,
        CampaignArtifactKind::TrainingCheckpoint,
        checkpoint.id,
        artifact_core::fingerprint(checkpoint)?,
        CampaignLinkDetails::TrainingCheckpoint {
            training_run_id: checkpoint.run_id,
        },
    )
}

pub fn link_candidate_evaluation(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    evaluation: &EvaluationRun,
) -> Result<CampaignArtifactLink, CampaignError> {
    require_link(
        campaign,
        links,
        CampaignArtifactKind::TrainingCheckpoint,
        evaluation.checkpoint_id,
    )?;
    let evaluates_baseline_cohort = evaluation.snapshot_id == campaign.source_snapshot_id
        && evaluation.source_identity.snapshot_fingerprint == campaign.source_snapshot_fingerprint;
    if !evaluates_baseline_cohort {
        require_link(
            campaign,
            links,
            CampaignArtifactKind::DatasetSnapshot,
            evaluation.snapshot_id,
        )?;
    }
    if evaluation.state != EvaluationRunState::Completed
        || evaluation
            .source_identity
            .snapshot_fingerprint
            .trim()
            .is_empty()
        || evaluation
            .source_identity
            .cohort_fingerprint
            .trim()
            .is_empty()
    {
        return Err(CampaignError::EvaluationCompatibility);
    }
    make_link(
        campaign,
        CampaignArtifactKind::CandidateEvaluation,
        evaluation.id,
        artifact_core::fingerprint(evaluation)?,
        CampaignLinkDetails::CandidateEvaluation {
            checkpoint_id: evaluation.checkpoint_id,
            snapshot_id: evaluation.snapshot_id,
            cohort_fingerprint: evaluation.source_identity.cohort_fingerprint.clone(),
            protocol_fingerprint: evaluation.protocol_fingerprint.clone(),
        },
    )
}

pub fn link_comparison(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    comparison: &EvaluationComparisonReport,
) -> Result<CampaignArtifactLink, CampaignError> {
    require_link(
        campaign,
        links,
        CampaignArtifactKind::CandidateEvaluation,
        comparison.right_run_id,
    )?;
    let candidate = links
        .iter()
        .find(|link| {
            link.artifact_kind == CampaignArtifactKind::CandidateEvaluation
                && link.artifact_id == comparison.right_run_id
        })
        .ok_or(CampaignError::MissingLink)?;
    let CampaignLinkDetails::CandidateEvaluation {
        cohort_fingerprint,
        protocol_fingerprint,
        ..
    } = &candidate.details
    else {
        return Err(CampaignError::ComparisonCompatibility);
    };
    if comparison.left_run_id != campaign.baseline_evaluation_run_id
        || comparison.cohort_fingerprint != *cohort_fingerprint
        || comparison.protocol_fingerprint != *protocol_fingerprint
        || evaluation_core::comparison::comparison_fingerprint(comparison)?
            != comparison.fingerprint
    {
        return Err(CampaignError::ComparisonCompatibility);
    }
    make_link(
        campaign,
        CampaignArtifactKind::EvaluationComparison,
        comparison.id,
        comparison.fingerprint.clone(),
        CampaignLinkDetails::EvaluationComparison {
            baseline_evaluation_run_id: comparison.left_run_id,
            candidate_evaluation_run_id: comparison.right_run_id,
            cohort_fingerprint: comparison.cohort_fingerprint.clone(),
            protocol_fingerprint: comparison.protocol_fingerprint.clone(),
        },
    )
}

pub fn link_follow_up_analysis(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    report: &AnalysisReport,
) -> Result<CampaignArtifactLink, CampaignError> {
    let source = report
        .source_identity
        .as_ref()
        .ok_or(CampaignError::AnalysisCompatibility)?;
    require_link(
        campaign,
        links,
        CampaignArtifactKind::CandidateEvaluation,
        source.evaluation_run_id,
    )?;
    if let Some(comparison_id) = source.comparison_id {
        require_link(
            campaign,
            links,
            CampaignArtifactKind::EvaluationComparison,
            comparison_id,
        )?;
    }
    if analysis_core::runner::reproduce_report_fingerprint(report)
        .map_err(|error| CampaignError::AnalysisFingerprint(error.to_string()))?
        != report.fingerprint
    {
        return Err(CampaignError::AnalysisCompatibility);
    }
    make_link(
        campaign,
        CampaignArtifactKind::FollowUpAnalysis,
        report.id,
        report.fingerprint.clone(),
        CampaignLinkDetails::FollowUpAnalysis {
            evaluation_run_id: source.evaluation_run_id,
            comparison_id: source.comparison_id,
        },
    )
}

pub fn assess_campaign_outcome(
    campaign: &OptimizationCampaign,
    proposal: &OptimizationProposal,
    links: &[CampaignArtifactLink],
    comparison: &EvaluationComparisonReport,
    observed_accepted_by_cell: &BTreeMap<String, u32>,
    policy: OutcomeAssessmentPolicy,
) -> Result<CampaignOutcomeAssessment, CampaignError> {
    campaign.validate()?;
    validate_policy(policy)?;
    if campaign.proposal_id != proposal.id || campaign.proposal_fingerprint != proposal.fingerprint
    {
        return Err(CampaignError::ProposalIdentity);
    }
    require_link(
        campaign,
        links,
        CampaignArtifactKind::EvaluationComparison,
        comparison.id,
    )?;
    let mut target_coverage = proposal
        .normalized_recommendations
        .iter()
        .filter(|recommendation| {
            campaign
                .selected_recommendation_ids
                .contains(&recommendation.id)
        })
        .map(|recommendation| {
            let observed = observed_accepted_by_cell
                .get(&recommendation.cell.key())
                .copied()
                .ok_or(CampaignError::MissingCoverage)?;
            Ok(TargetCoverageOutcome {
                cell_key: recommendation.cell.key(),
                baseline_accepted: recommendation.current_accepted,
                proposed_target: recommendation.proposed_target,
                observed_accepted: observed,
                realized: observed >= recommendation.proposed_target,
            })
        })
        .collect::<Result<Vec<_>, CampaignError>>()?;
    target_coverage.sort_by(|left, right| left.cell_key.cmp(&right.cell_key));
    let constraints_realized = target_coverage.iter().all(|outcome| outcome.realized);
    let classification = classify_outcome(comparison, policy)?;
    let created_at = Utc::now();
    let policy_fingerprint = artifact_core::fingerprint(&policy)?;
    let mut assessment = CampaignOutcomeAssessment {
        id: Uuid::new_v4(),
        campaign_id: campaign.id,
        comparison_id: comparison.id,
        comparison_fingerprint: comparison.fingerprint.clone(),
        policy,
        policy_fingerprint,
        accuracy_delta: comparison.accuracy_delta,
        macro_f1_delta: comparison.macro_f1_delta,
        per_label_f1_delta: comparison.per_label_f1_delta.clone(),
        accuracy_delta_interval: comparison.accuracy_delta_interval.clone(),
        macro_f1_delta_interval: comparison.macro_f1_delta_interval.clone(),
        mcnemar_p_value: comparison.mcnemar.two_sided_p_value,
        mcnemar_significant: comparison.mcnemar.significant,
        fixed_errors: comparison.right_only_correct,
        regressed_errors: comparison.left_only_correct,
        persistent_errors: comparison.both_wrong,
        target_coverage,
        constraints_realized,
        classification,
        caution: "observed association only; this campaign does not establish causal attribution"
            .into(),
        fingerprint: String::new(),
        created_at,
    };
    assessment.fingerprint = reproduce_fingerprint(&assessment)?;
    Ok(assessment)
}

pub fn classify_outcome(
    comparison: &EvaluationComparisonReport,
    policy: OutcomeAssessmentPolicy,
) -> Result<OutcomeClassification, CampaignError> {
    validate_policy(policy)?;
    let values = [
        comparison.accuracy_delta,
        comparison.macro_f1_delta,
        comparison.accuracy_delta_interval.lower,
        comparison.accuracy_delta_interval.upper,
        comparison.macro_f1_delta_interval.lower,
        comparison.macro_f1_delta_interval.upper,
        comparison.mcnemar.two_sided_p_value,
    ];
    if values.iter().any(|value| !value.is_finite()) {
        return Err(CampaignError::ComparisonMetrics);
    }
    let accuracy_improved = comparison.accuracy_delta > policy.minimum_accuracy_delta;
    let f1_improved = comparison.macro_f1_delta > policy.minimum_macro_f1_delta;
    let accuracy_regressed = comparison.accuracy_delta < -policy.minimum_accuracy_delta;
    let f1_regressed = comparison.macro_f1_delta < -policy.minimum_macro_f1_delta;
    let significant_improvement = !policy.require_significance
        || comparison.mcnemar.significant
            && comparison.accuracy_delta_interval.lower > 0.0
            && comparison.macro_f1_delta_interval.lower > 0.0;
    let significant_regression = !policy.require_significance
        || comparison.mcnemar.significant
            && comparison.accuracy_delta_interval.upper < 0.0
            && comparison.macro_f1_delta_interval.upper < 0.0;
    Ok(
        if accuracy_improved && f1_improved && significant_improvement {
            OutcomeClassification::Improved
        } else if accuracy_regressed && f1_regressed && significant_regression {
            OutcomeClassification::Regressed
        } else if comparison.accuracy_delta.signum() != comparison.macro_f1_delta.signum()
            || comparison.left_only_correct > 0 && comparison.right_only_correct > 0
        {
            OutcomeClassification::Mixed
        } else {
            OutcomeClassification::Inconclusive
        },
    )
}

fn make_link(
    campaign: &OptimizationCampaign,
    artifact_kind: CampaignArtifactKind,
    artifact_id: Uuid,
    artifact_fingerprint: String,
    details: CampaignLinkDetails,
) -> Result<CampaignArtifactLink, CampaignError> {
    validate_campaign(campaign)?;
    let mut link = CampaignArtifactLink {
        id: Uuid::new_v4(),
        campaign_id: campaign.id,
        artifact_kind,
        artifact_id,
        artifact_fingerprint,
        details,
        fingerprint: String::new(),
        created_at: Utc::now(),
    };
    link.fingerprint = reproduce_fingerprint(&link)?;
    Ok(link)
}

fn require_link(
    campaign: &OptimizationCampaign,
    links: &[CampaignArtifactLink],
    kind: CampaignArtifactKind,
    artifact_id: Uuid,
) -> Result<(), CampaignError> {
    let link = links
        .iter()
        .find(|link| link.artifact_kind == kind && link.artifact_id == artifact_id)
        .ok_or(CampaignError::MissingLink)?;
    if link.campaign_id != campaign.id || reproduce_fingerprint(link)? != link.fingerprint {
        return Err(CampaignError::LinkFingerprint);
    }
    Ok(())
}

fn validate_campaign(campaign: &OptimizationCampaign) -> Result<(), CampaignError> {
    if reproduce_fingerprint(campaign)? == campaign.fingerprint {
        Ok(())
    } else {
        Err(CampaignError::CampaignFingerprint)
    }
}

fn validate_policy(policy: OutcomeAssessmentPolicy) -> Result<(), CampaignError> {
    if !policy.minimum_accuracy_delta.is_finite()
        || !policy.minimum_macro_f1_delta.is_finite()
        || policy.minimum_accuracy_delta < 0.0
        || policy.minimum_macro_f1_delta < 0.0
    {
        return Err(CampaignError::AssessmentPolicy);
    }
    Ok(())
}

fn reproduce_fingerprint<T>(value: &T) -> Result<String, artifact_core::FingerprintError>
where
    T: Serialize + Clone + ClearFingerprint,
{
    let mut input = value.clone();
    input.clear_fingerprint();
    artifact_core::fingerprint(&input)
}

trait ClearFingerprint {
    fn clear_fingerprint(&mut self);
}

impl ClearFingerprint for OptimizationCampaign {
    fn clear_fingerprint(&mut self) {
        self.fingerprint.clear();
    }
}

impl ClearFingerprint for CampaignArtifactLink {
    fn clear_fingerprint(&mut self) {
        self.fingerprint.clear();
    }
}

impl ClearFingerprint for CampaignOutcomeAssessment {
    fn clear_fingerprint(&mut self) {
        self.fingerprint.clear();
    }
}

#[derive(Debug, Error)]
pub enum CampaignError {
    #[error(transparent)]
    Proposal(#[from] crate::planning::OptimizationError),
    #[error(transparent)]
    Review(#[from] crate::reviews::ProposalReviewError),
    #[error(transparent)]
    Fingerprint(#[from] artifact_core::FingerprintError),
    #[error("campaign requires an approval or accepted training candidate review")]
    ApprovalState,
    #[error("campaign requires a decision-grade proposal")]
    LegacyProposal,
    #[error("campaign fingerprint does not reproduce")]
    CampaignFingerprint,
    #[error("campaign artifact link fingerprint or campaign identity does not reproduce")]
    LinkFingerprint,
    #[error(
        "campaign outcome fingerprint, policy, comparison, or classification does not reproduce"
    )]
    OutcomeFingerprint,
    #[error("generation plan is incompatible with the campaign application")]
    GenerationPlanCompatibility,
    #[error("generation job is incompatible with the campaign")]
    GenerationJobCompatibility,
    #[error("snapshot metadata is incompatible with the campaign")]
    SnapshotCompatibility,
    #[error("snapshot generated-row lineage does not resolve through linked jobs")]
    SnapshotLineage,
    #[error("required prior campaign link is missing")]
    MissingLink,
    #[error("evaluation is incomplete or has incomplete immutable source identity")]
    EvaluationCompatibility,
    #[error("comparison does not pair the campaign baseline with its candidate")]
    ComparisonCompatibility,
    #[error("follow-up analysis does not reference the linked candidate evidence")]
    AnalysisCompatibility,
    #[error("follow-up analysis fingerprint could not be reproduced: {0}")]
    AnalysisFingerprint(String),
    #[error("campaign and proposal identities differ")]
    ProposalIdentity,
    #[error("outcome assessment requires accepted coverage for every selected data target")]
    MissingCoverage,
    #[error("outcome-assessment thresholds must be finite and non-negative")]
    AssessmentPolicy,
    #[error("comparison metrics used for outcome assessment must be finite")]
    ComparisonMetrics,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use evaluation_core::domain::{
        ClassificationMetrics, ConfidenceInterval, EvaluationComparisonReport, EvaluationMetrics,
        McNemarResult,
    };
    use uuid::Uuid;

    use crate::domain::OptimizationProposal;

    use super::{
        CampaignArtifactKind, CampaignLinkDetails, OptimizationCampaign, OutcomeAssessmentPolicy,
        OutcomeClassification, assess_campaign_outcome, classify_outcome, make_link,
        reproduce_fingerprint,
    };

    #[test]
    fn classifies_persisted_paired_evidence_deterministically() {
        let metrics = EvaluationMetrics {
            overall: empty_metrics(),
            by_dimension: BTreeMap::new(),
            slices: BTreeMap::new(),
        };
        let mut comparison = EvaluationComparisonReport {
            id: Uuid::new_v4(),
            left_run_id: Uuid::new_v4(),
            right_run_id: Uuid::new_v4(),
            cohort_fingerprint: "sha256:cohort".into(),
            protocol_fingerprint: "sha256:protocol".into(),
            left_metrics: metrics.clone(),
            right_metrics: metrics,
            accuracy_delta: 0.05,
            macro_f1_delta: 0.04,
            per_label_f1_delta: BTreeMap::from([("billing".into(), 0.04)]),
            both_correct: 80,
            both_wrong: 8,
            left_only_correct: 2,
            right_only_correct: 10,
            fixed_snapshot_member_ids: Vec::new(),
            regressed_snapshot_member_ids: Vec::new(),
            accuracy_delta_interval: ConfidenceInterval {
                level: 0.95,
                lower: 0.01,
                upper: 0.09,
            },
            macro_f1_delta_interval: ConfidenceInterval {
                level: 0.95,
                lower: 0.005,
                upper: 0.08,
            },
            mcnemar: McNemarResult {
                left_only_correct: 2,
                right_only_correct: 10,
                two_sided_p_value: 0.02,
                significant: true,
            },
            slice_deltas: BTreeMap::new(),
            fingerprint: String::new(),
            created_at: Utc::now(),
        };
        comparison.fingerprint = evaluation_core::comparison::comparison_fingerprint(&comparison)
            .expect("comparison fingerprint");
        let policy = OutcomeAssessmentPolicy {
            minimum_accuracy_delta: 0.01,
            minimum_macro_f1_delta: 0.01,
            require_significance: true,
        };
        assert_eq!(
            classify_outcome(&comparison, policy).expect("classification"),
            OutcomeClassification::Improved
        );
        let proposal = OptimizationProposal {
            id: Uuid::new_v4(),
            analysis_report_id: Uuid::new_v4(),
            dataset_id: Uuid::new_v4(),
            additional_example_budget: 1,
            minimum_support: 1,
            protocol: None,
            protocol_fingerprint: String::new(),
            source_identity: None,
            evidence_fingerprint: String::new(),
            decision_evidence: None,
            allocation: None,
            decision_cells: Vec::new(),
            normalized_recommendations: Vec::new(),
            training_candidate_set: None,
            review_only_recommendations: Vec::new(),
            rebase_lineage: None,
            recommendations: Vec::new(),
            fingerprint: "sha256:proposal".into(),
            created_at: Utc::now(),
        };
        let mut campaign = OptimizationCampaign {
            id: Uuid::new_v4(),
            proposal_id: proposal.id,
            proposal_fingerprint: proposal.fingerprint.clone(),
            approval_review_id: Uuid::new_v4(),
            approval_fingerprint: "sha256:approval".into(),
            selected_recommendation_ids: Vec::new(),
            baseline_analysis_report_id: proposal.analysis_report_id,
            baseline_analysis_fingerprint: "sha256:analysis".into(),
            baseline_evaluation_run_id: comparison.left_run_id,
            baseline_comparison_id: None,
            dataset_id: proposal.dataset_id,
            source_snapshot_id: Uuid::new_v4(),
            source_snapshot_fingerprint: "sha256:snapshot".into(),
            fingerprint: String::new(),
            created_at: Utc::now(),
        };
        campaign.fingerprint = reproduce_fingerprint(&campaign).expect("campaign fingerprint");
        let comparison_link = make_link(
            &campaign,
            CampaignArtifactKind::EvaluationComparison,
            comparison.id,
            comparison.fingerprint.clone(),
            CampaignLinkDetails::EvaluationComparison {
                baseline_evaluation_run_id: comparison.left_run_id,
                candidate_evaluation_run_id: comparison.right_run_id,
                cohort_fingerprint: comparison.cohort_fingerprint.clone(),
                protocol_fingerprint: comparison.protocol_fingerprint.clone(),
            },
        )
        .expect("comparison link");
        let assessment = assess_campaign_outcome(
            &campaign,
            &proposal,
            std::slice::from_ref(&comparison_link),
            &comparison,
            &BTreeMap::new(),
            policy,
        )
        .expect("campaign assessment");
        assert_eq!(assessment.classification, OutcomeClassification::Improved);
        assessment
            .validate(&campaign, &comparison)
            .expect("assessment validates");
        assessment
            .validate_reproduction(
                &campaign,
                &proposal,
                std::slice::from_ref(&comparison_link),
                &comparison,
                &BTreeMap::new(),
            )
            .expect("assessment reproduces from comparison facts");
        let mut tampered = assessment;
        tampered.accuracy_delta += 0.1;
        tampered.fingerprint = reproduce_fingerprint(&tampered).expect("tampered fingerprint");
        assert!(
            tampered
                .validate_reproduction(
                    &campaign,
                    &proposal,
                    std::slice::from_ref(&comparison_link),
                    &comparison,
                    &BTreeMap::new(),
                )
                .is_err()
        );

        let mut mixed = comparison;
        mixed.macro_f1_delta = -0.02;
        assert_eq!(
            classify_outcome(&mixed, policy).expect("mixed classification"),
            OutcomeClassification::Mixed
        );
    }

    fn empty_metrics() -> ClassificationMetrics {
        ClassificationMetrics {
            total: 0,
            correct: 0,
            accuracy: 0.0,
            macro_precision: 0.0,
            macro_recall: 0.0,
            macro_f1: 0.0,
            weighted_precision: 0.0,
            weighted_recall: 0.0,
            weighted_f1: 0.0,
            top_k_accuracy: BTreeMap::new(),
            log_loss: 0.0,
            brier_score: 0.0,
            expected_calibration_error: 0.0,
            mean_confidence: 0.0,
            mean_correct_confidence: 0.0,
            mean_incorrect_confidence: 0.0,
            per_label: BTreeMap::new(),
            confusion_matrix: BTreeMap::new(),
        }
    }
}
