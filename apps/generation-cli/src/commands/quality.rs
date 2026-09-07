use std::{collections::BTreeMap, sync::Arc, time::Duration};

use anyhow::{Context, ensure};
use dataset_quality_core::{
    assessment::{
        AuthenticityEvaluatorGuidance, ConceptGuidance, EvaluatorGuidance, EvaluatorIdentity,
        EvaluatorIndependence, QualityVerdict, SemanticEvaluatorGuidance, SemanticTargetGuidance,
    },
    curation::{
        ApprovedCurationManifest, ArtifactReference, CurationCounts, CurationDisposition,
        CurationManifestReview, CurationManifestReviewDecision, CurationProposal,
        DatasetQualityReport, QualityCounts, RowQualityReview, RowQualityReviewDecision,
    },
    lifecycle::{
        AuditPlanStatusProjection, AuditProgress, AuditUsage, QualityAuditRun,
        QualityAuditRunState, QualityAuditStopReason,
    },
    policy::{
        AuditMode, EvaluatorEgressPolicy, QualityPolicy, QualityPolicyPresetControls, QualityPreset,
    },
    population::{AuditPlan, GuidanceReference, GuidanceReferences},
    ports::{DatasetQualityStore, QualityCandidateSource, QualityEvaluator},
};
use dataset_quality_fake::{
    DEFAULT_EVALUATOR_PROTOCOL, FAKE_QUALITY_BACKEND, FakeQualityEvaluator,
};
use dataset_quality_openai_compatible::{
    OpenAICompatibleEvaluatorConfig, OpenAICompatibleQualityEvaluator,
};
use dataset_quality_runner::{DatasetQualityRunner, QualityAuditOutcome};
use generation_core::ports::{BackendConfigurationStore, DatasetStore};
use research_core::{ports::ResearchStore, profile::ResolvedAuthenticityContext};
use semantic_catalog::{ResolvedSemanticContext, ResolvedSemanticTarget, SemanticTarget};
use serde::Serialize;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::{
    cli::{
        QualityAssessmentsArgs, QualityAuditCreateArgs, QualityAuditExecutionArgs,
        QualityAuthenticityArg, QualityCommand, QualityEgressArg, QualityEvaluatorArg,
        QualityManifestReviewArgs, QualityPolicyArgs, QualityPresetArg, QualityRowReviewArgs,
        QualityVerdictArg,
    },
    presentation,
};

const OPENAI_COMPATIBLE_QUALITY_BACKEND: &str = "openai-compatible";
const PRIMARY_EVALUATOR_SEED: i64 = 42;
const REVIEWER_EVALUATOR_SEED_BASE: i64 = 1_001;

pub async fn execute(command: QualityCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        QualityCommand::PolicyPreview(args) => presentation::print(&compile_policy(&args)?),
        QualityCommand::AuditCreate(args) => create_audit(args, store).await,
        QualityCommand::AuditStart(args) | QualityCommand::AuditRecover(args) => {
            run_audit(args, store).await
        }
        QualityCommand::AuditStatus { run_id } => {
            presentation::print(&audit_status(store, run_id).await?)
        }
        QualityCommand::AuditWatch { run_id, poll_ms } => watch_audit(store, run_id, poll_ms).await,
        QualityCommand::AuditCancel { run_id } => cancel_audit(store, run_id).await,
        QualityCommand::Assessments(args) => assessments(store, args).await,
        QualityCommand::Summary { run_id } => summary(store, run_id).await,
        QualityCommand::Curate { run_id } => curate(store, run_id).await,
        QualityCommand::RowReview(args) => row_review(store, args).await,
        QualityCommand::Proposal { proposal_id } => show_proposal(store, proposal_id).await,
        QualityCommand::ManifestReview(args) => manifest_review(store, args).await,
        QualityCommand::Manifest { manifest_id } => show_manifest(store, manifest_id).await,
    }
}

pub(super) fn compile_policy(args: &QualityPolicyArgs) -> anyhow::Result<QualityPolicy> {
    let preset = match args.preset {
        QualityPresetArg::Fast => QualityPreset::Fast,
        QualityPresetArg::Balanced => QualityPreset::Balanced,
        QualityPresetArg::Strict => QualityPreset::Strict,
    };
    let egress_policy = match args.egress {
        QualityEgressArg::LocalOnly => EvaluatorEgressPolicy::LocalOnly,
        QualityEgressArg::ExternalCandidateText => EvaluatorEgressPolicy::ExternalCandidateText,
    };
    preset
        .compile(QualityPolicyPresetControls {
            audit_mode: AuditMode::FullPopulation,
            egress_policy,
            evaluate_authenticity: args.authenticity != QualityAuthenticityArg::Off,
            maximum_cost_microusd: args.max_cost_microusd,
        })
        .map_err(Into::into)
}

async fn create_audit(args: QualityAuditCreateArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let dataset = store
        .get_dataset(args.dataset_id)
        .await?
        .with_context(|| format!("dataset not found: {}", args.dataset_id))?;
    let source_rows = store.list_source_rows(dataset.id).await?;
    let plan_id = Uuid::new_v4();
    let (guidance_references, guidance) =
        resolve_create_guidance(store, dataset.id, plan_id, args.policy.authenticity).await?;
    let mut policy_args = args.policy;
    if policy_args.authenticity == QualityAuthenticityArg::WhenAvailable
        && guidance.authenticity.is_none()
    {
        policy_args.authenticity = QualityAuthenticityArg::Off;
    }
    let policy = compile_policy(&policy_args)?;
    let plan = AuditPlan::with_identity(
        plan_id,
        &dataset,
        policy,
        guidance_references,
        guidance.reproduce_fingerprint()?,
        DEFAULT_EVALUATOR_PROTOCOL,
        source_rows,
        chrono::Utc::now(),
    )?;
    if args.evaluator == QualityEvaluatorArg::OpenaiCompatible {
        ensure!(
            plan.policy
                .borderline_review_policy
                .required_additional_assessments()
                == 0,
            "openai-compatible quality audits currently support only --preset fast; balanced and strict require distinct reviewer model or backend configurations, which are not yet supported"
        );
        ensure!(
            plan.policy.budgets.maximum_cost_microusd.is_none(),
            "openai-compatible quality audits cannot use --max-cost-microusd until provider token pricing is explicitly pinned"
        );
    }
    let evaluator_bundle = evaluator_bundle(
        args.evaluator,
        plan.policy
            .borderline_review_policy
            .required_additional_assessments(),
        store,
    )
    .await?;
    let run = QualityAuditRun::queue(&plan, evaluator_bundle.primary, evaluator_bundle.reviewers)?;
    store.create_audit(&plan, &run, &guidance).await?;
    presentation::print(&serde_json::json!({
        "audit_plan": AuditPlanSummary::from(&plan),
        "audit_run": QualityAuditRunSummary::from(&run),
    }))
}

async fn run_audit(args: QualityAuditExecutionArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let run = require_run(store, args.run_id).await?;
    let adapters = evaluators_for_run(&run, store, &args.api_key_env).await?;
    let runner =
        DatasetQualityRunner::new(Arc::new(store.clone()), Arc::new(store.clone()), adapters)?;
    let outcome = runner.execute(args.run_id).await?;
    presentation::print(&QualityAuditOutcomeSummary {
        run: QualityAuditRunSummary::from(&outcome.run),
        report: outcome
            .report
            .as_ref()
            .map(DatasetQualityReportSummary::from),
    })
}

async fn cancel_audit(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    let mut run = require_run(store, run_id).await?;
    ensure!(
        matches!(
            run.state,
            QualityAuditRunState::Queued | QualityAuditRunState::Running
        ),
        "cannot cancel a {:?} quality audit",
        run.state
    );
    if run.state == QualityAuditRunState::Queued {
        run.cancel()?;
    } else {
        run.request_cancel()?;
    }
    store.save_audit_run(&run).await?;
    presentation::print(&run)
}

#[derive(Debug, Serialize)]
struct QualityAuditStatus {
    run: QualityAuditRunSummary,
    plan: AuditPlanSummary,
    evaluator_requests: usize,
    evaluator_attempts: usize,
    assessments: usize,
    report_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct AuditPlanSummary {
    id: Uuid,
    schema_version: u32,
    dataset_definition_id: Uuid,
    dataset_definition_fingerprint: String,
    policy_fingerprint: String,
    resolved_guidance_fingerprint: String,
    evaluator_protocol_version: String,
    population_rows: u64,
    selected_rows: u64,
    source_set_fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
    fingerprint: String,
}

impl From<&AuditPlan> for AuditPlanSummary {
    fn from(plan: &AuditPlan) -> Self {
        Self {
            id: plan.id,
            schema_version: plan.schema_version,
            dataset_definition_id: plan.dataset_schema.dataset_definition_id,
            dataset_definition_fingerprint: plan
                .dataset_schema
                .dataset_definition_fingerprint
                .clone(),
            policy_fingerprint: plan.policy.fingerprint.clone(),
            resolved_guidance_fingerprint: plan.resolved_guidance_fingerprint.clone(),
            evaluator_protocol_version: plan.evaluator_protocol_version.clone(),
            population_rows: plan.population_count(),
            selected_rows: plan.selected_count(),
            source_set_fingerprint: plan.source_set_fingerprint.clone(),
            created_at: plan.created_at,
            fingerprint: plan.fingerprint.clone(),
        }
    }
}

impl From<&AuditPlanStatusProjection> for AuditPlanSummary {
    fn from(plan: &AuditPlanStatusProjection) -> Self {
        Self {
            id: plan.id,
            schema_version: plan.schema_version,
            dataset_definition_id: plan.dataset_definition_id,
            dataset_definition_fingerprint: plan.dataset_definition_fingerprint.clone(),
            policy_fingerprint: plan.policy_fingerprint.clone(),
            resolved_guidance_fingerprint: plan.resolved_guidance_fingerprint.clone(),
            evaluator_protocol_version: plan.evaluator_protocol_version.clone(),
            population_rows: plan.population_rows,
            selected_rows: plan.selected_rows,
            source_set_fingerprint: plan.source_set_fingerprint.clone(),
            created_at: plan.created_at,
            fingerprint: plan.fingerprint.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct QualityAuditRunSummary {
    id: Uuid,
    schema_version: u32,
    plan_id: Uuid,
    plan_fingerprint: String,
    source_set_fingerprint: String,
    policy_fingerprint: String,
    primary_evaluator: EvaluatorIdentity,
    independent_reviewers: Vec<EvaluatorIdentity>,
    independent_reviewer_count: usize,
    state: QualityAuditRunState,
    progress: AuditProgress,
    usage: AuditUsage,
    cancel_requested: bool,
    stop_reason: Option<QualityAuditStopReason>,
    error_message: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    specification_fingerprint: String,
}

impl From<&QualityAuditRun> for QualityAuditRunSummary {
    fn from(run: &QualityAuditRun) -> Self {
        Self {
            id: run.id,
            schema_version: run.schema_version,
            plan_id: run.plan_id,
            plan_fingerprint: run.plan_fingerprint.clone(),
            source_set_fingerprint: run.source_set_fingerprint.clone(),
            policy_fingerprint: run.policy_fingerprint.clone(),
            primary_evaluator: run.primary_evaluator.clone(),
            independent_reviewers: run.independent_reviewers.clone(),
            independent_reviewer_count: run.independent_reviewers.len(),
            state: run.state,
            progress: run.progress,
            usage: run.usage,
            cancel_requested: run.cancel_requested,
            stop_reason: run.stop_reason,
            error_message: run.error_message.clone(),
            created_at: run.created_at,
            started_at: run.started_at,
            finished_at: run.finished_at,
            specification_fingerprint: run.specification_fingerprint.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct QualityAuditOutcomeSummary {
    run: QualityAuditRunSummary,
    report: Option<DatasetQualityReportSummary>,
}

#[derive(Debug, Serialize)]
struct DatasetQualityReportSummary {
    id: Uuid,
    schema_version: u32,
    plan_id: Uuid,
    plan_fingerprint: String,
    run_id: Uuid,
    run_specification_fingerprint: String,
    dataset_definition_id: Uuid,
    dataset_definition_fingerprint: String,
    source_set_fingerprint: String,
    policy_fingerprint: String,
    row_count: usize,
    assessment_set_fingerprint: String,
    invalid_attempt_set_fingerprint: String,
    totals: QualityCounts,
    label_summary_count: usize,
    cell_summary_count: usize,
    provenance_summary_count: usize,
    issue_summary_count: usize,
    created_at: chrono::DateTime<chrono::Utc>,
    fingerprint: String,
}

impl From<&DatasetQualityReport> for DatasetQualityReportSummary {
    fn from(report: &DatasetQualityReport) -> Self {
        Self {
            id: report.id,
            schema_version: report.schema_version,
            plan_id: report.plan_id,
            plan_fingerprint: report.plan_fingerprint.clone(),
            run_id: report.run_id,
            run_specification_fingerprint: report.run_specification_fingerprint.clone(),
            dataset_definition_id: report.dataset_definition_id,
            dataset_definition_fingerprint: report.dataset_definition_fingerprint.clone(),
            source_set_fingerprint: report.source_set_fingerprint.clone(),
            policy_fingerprint: report.policy_fingerprint.clone(),
            row_count: report.rows.len(),
            assessment_set_fingerprint: report.assessment_set_fingerprint.clone(),
            invalid_attempt_set_fingerprint: report.invalid_attempt_set_fingerprint.clone(),
            totals: report.totals,
            label_summary_count: report.by_label.len(),
            cell_summary_count: report.by_cell.len(),
            provenance_summary_count: report.by_provenance.len(),
            issue_summary_count: report.by_issue.len(),
            created_at: report.created_at,
            fingerprint: report.fingerprint.clone(),
        }
    }
}

async fn audit_status(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<QualityAuditStatus> {
    let status = store
        .get_audit_status(run_id)
        .await?
        .with_context(|| format!("quality audit run not found: {run_id}"))?;
    Ok(QualityAuditStatus {
        run: QualityAuditRunSummary::from(&status.run),
        plan: AuditPlanSummary::from(&status.plan),
        evaluator_requests: usize::try_from(status.evaluator_requests)
            .context("quality evaluator request count exceeds this platform")?,
        evaluator_attempts: usize::try_from(status.evaluator_attempts)
            .context("quality evaluator attempt count exceeds this platform")?,
        assessments: usize::try_from(status.assessments)
            .context("quality assessment count exceeds this platform")?,
        report_id: status.report_id,
    })
}

async fn watch_audit(store: &SqliteStore, run_id: Uuid, poll_ms: u64) -> anyhow::Result<()> {
    loop {
        let status = audit_status(store, run_id).await?;
        eprintln!(
            "quality audit {}: {:?}, assessed={}, qualified={}, borderline={}, quarantined={}, invalid={}, requests={}",
            run_id,
            status.run.state,
            status.run.progress.assessed_rows,
            status.run.progress.qualified_rows,
            status.run.progress.borderline_rows,
            status.run.progress.quarantined_rows,
            status.run.progress.invalid_rows,
            status.evaluator_requests,
        );
        if !matches!(
            status.run.state,
            QualityAuditRunState::Queued | QualityAuditRunState::Running
        ) {
            return presentation::print(&status);
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn assessments(store: &SqliteStore, args: QualityAssessmentsArgs) -> anyhow::Result<()> {
    require_run(store, args.run_id).await?;
    let expected = args.verdict.map(quality_verdict);
    let values = store
        .list_assessments(args.run_id)
        .await?
        .into_iter()
        .filter(|assessment| expected.is_none_or(|value| assessment.verdict == value))
        .skip(args.page.offset as usize)
        .take(args.page.limit as usize)
        .collect::<Vec<_>>();
    presentation::print_page(&values, values.len(), args.page)
}

async fn summary(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    let report = require_verified_report(store, run_id).await?;
    let remaining_qualified_rows = report
        .totals
        .population_rows
        .checked_sub(report.totals.qualified_rows)
        .context("verified quality totals have more qualified rows than population rows")?;
    presentation::print(&serde_json::json!({
        "report_id": report.id,
        "run_id": report.run_id,
        "structurally_accepted_rows": report.totals.population_rows,
        "population_rows": report.totals.population_rows,
        "assessed_rows": report.totals.assessed_rows,
        "qualified_rows": report.totals.qualified_rows,
        "remaining_qualified_rows": remaining_qualified_rows,
        "totals": report.totals,
        "by_label": report.by_label,
        "by_cell": report.by_cell,
        "by_provenance": report.by_provenance,
        "by_issue": report.by_issue,
    }))
}

#[derive(Debug, Serialize)]
struct CurationProposalSummary {
    id: Uuid,
    schema_version: u32,
    report_id: Uuid,
    report_fingerprint: String,
    dataset_definition_id: Uuid,
    dataset_definition_fingerprint: String,
    predecessor_id: Option<Uuid>,
    predecessor_fingerprint: Option<String>,
    row_review_set_fingerprint: String,
    row_review_count: usize,
    entry_count: usize,
    counts: CurationCounts,
    created_at: chrono::DateTime<chrono::Utc>,
    fingerprint: String,
}

impl From<&CurationProposal> for CurationProposalSummary {
    fn from(proposal: &CurationProposal) -> Self {
        Self {
            id: proposal.id,
            schema_version: proposal.schema_version,
            report_id: proposal.report_id,
            report_fingerprint: proposal.report_fingerprint.clone(),
            dataset_definition_id: proposal.dataset_definition_id,
            dataset_definition_fingerprint: proposal.dataset_definition_fingerprint.clone(),
            predecessor_id: proposal.predecessor_id,
            predecessor_fingerprint: proposal.predecessor_fingerprint.clone(),
            row_review_set_fingerprint: proposal.row_review_set_fingerprint.clone(),
            row_review_count: proposal.row_review_references.len(),
            entry_count: proposal.entries.len(),
            counts: proposal.counts,
            created_at: proposal.created_at,
            fingerprint: proposal.fingerprint.clone(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ApprovedManifestSummary {
    id: Uuid,
    schema_version: u32,
    report_id: Uuid,
    report_fingerprint: String,
    proposal_id: Uuid,
    proposal_fingerprint: String,
    approval_id: Uuid,
    approval_fingerprint: String,
    dataset_definition_id: Uuid,
    dataset_definition_fingerprint: String,
    member_count: usize,
    selected_member_count: usize,
    excluded_member_count: usize,
    complete_member_fingerprint: String,
    selected_member_fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
    fingerprint: String,
}

impl From<&ApprovedCurationManifest> for ApprovedManifestSummary {
    fn from(manifest: &ApprovedCurationManifest) -> Self {
        let selected_member_count = manifest
            .members
            .iter()
            .filter(|member| member.disposition == CurationDisposition::Include)
            .count();
        Self {
            id: manifest.id,
            schema_version: manifest.schema_version,
            report_id: manifest.report_id,
            report_fingerprint: manifest.report_fingerprint.clone(),
            proposal_id: manifest.proposal_id,
            proposal_fingerprint: manifest.proposal_fingerprint.clone(),
            approval_id: manifest.approval_id,
            approval_fingerprint: manifest.approval_fingerprint.clone(),
            dataset_definition_id: manifest.dataset_definition_id,
            dataset_definition_fingerprint: manifest.dataset_definition_fingerprint.clone(),
            member_count: manifest.members.len(),
            selected_member_count,
            excluded_member_count: manifest.members.len().saturating_sub(selected_member_count),
            complete_member_fingerprint: manifest.complete_member_fingerprint.clone(),
            selected_member_fingerprint: manifest.selected_member_fingerprint.clone(),
            created_at: manifest.created_at,
            fingerprint: manifest.fingerprint.clone(),
        }
    }
}

async fn curate(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    let proposal = create_or_reuse_curation_proposal(store, run_id).await?;
    presentation::print(&CurationProposalSummary::from(&proposal))
}

pub(crate) async fn create_or_reuse_curation_proposal(
    store: &SqliteStore,
    run_id: Uuid,
) -> anyhow::Result<CurationProposal> {
    let run = require_run(store, run_id).await?;
    ensure!(
        run.state == QualityAuditRunState::Completed,
        "quality audit {run_id} is not completed"
    );
    let report = require_verified_report(store, run_id).await?;
    let row_reviews = store.list_row_reviews(report.id).await?;
    let predecessor = store.latest_curation_proposal(report.id).await?;
    if let Some(existing) = &predecessor
        && existing.row_review_references == review_references(&row_reviews)
    {
        return Ok(existing.clone());
    }
    let proposal = CurationProposal::create(&report, predecessor.as_ref(), &row_reviews)?;
    store.save_curation_proposal(&proposal).await?;
    Ok(proposal)
}

async fn row_review(store: &SqliteStore, args: QualityRowReviewArgs) -> anyhow::Result<()> {
    let (report, source_row_id) = match (args.assessment_id, args.report_id, args.source_row_id) {
        (Some(assessment_id), None, None) => {
            let assessment = store
                .get_assessment(assessment_id)
                .await?
                .with_context(|| format!("quality assessment not found: {assessment_id}"))?;
            let report = require_verified_report(store, assessment.audit_run_id)
                .await
                .with_context(|| {
                    format!(
                        "assessment {} does not yet belong to a valid completed quality report",
                        assessment.id
                    )
                })?;
            let report_row = report.row(assessment.source_row_id).with_context(|| {
                format!(
                    "assessment {} targets a row outside its quality report",
                    assessment.id
                )
            })?;
            ensure!(
                report_row
                    .assessment_references
                    .iter()
                    .any(|reference| reference.id == assessment.id
                        && reference.fingerprint == assessment.fingerprint),
                "assessment {} is not immutable evidence for report {}",
                assessment.id,
                report.id
            );
            (report, assessment.source_row_id)
        }
        (None, Some(report_id), Some(source_row_id)) => {
            let report = require_verified_report_by_id(store, report_id).await?;
            ensure!(
                report.row(source_row_id).is_some(),
                "source row {source_row_id} is not a member of quality report {report_id}"
            );
            (report, source_row_id)
        }
        _ => anyhow::bail!(
            "row review requires either ASSESSMENT_ID or both --report-id and --source-row-id"
        ),
    };
    let reviews = store.list_row_reviews(report.id).await?;
    let predecessor = reviews
        .iter()
        .rev()
        .find(|review| review.source_row_id == source_row_id);
    let decision = if args.include {
        RowQualityReviewDecision::Include
    } else if args.exclude {
        RowQualityReviewDecision::Exclude
    } else {
        RowQualityReviewDecision::RequestReassessment
    };
    let review = RowQualityReview::create(
        &report,
        source_row_id,
        predecessor,
        decision,
        args.reviewer,
        args.reason,
    )?;
    store.append_row_review(&review).await?;
    presentation::print(&review)
}

async fn show_proposal(store: &SqliteStore, proposal_id: Uuid) -> anyhow::Result<()> {
    let evidence = load_proposal_evidence(store, proposal_id).await?;
    evidence.proposal.verify_against(
        &evidence.report,
        evidence.predecessor.as_ref(),
        &evidence.row_reviews,
    )?;
    presentation::print(&evidence.proposal)
}

async fn manifest_review(
    store: &SqliteStore,
    args: QualityManifestReviewArgs,
) -> anyhow::Result<()> {
    if let Some(existing) = store.manifest_for_proposal(args.proposal_id).await? {
        ensure!(
            args.approve,
            "proposal {} is already sealed by approved manifest {}",
            args.proposal_id,
            existing.id
        );
        let verified = load_verified_manifest(store, existing.id).await?;
        return presentation::print(&ApprovedManifestSummary::from(&verified.manifest));
    }
    let evidence = load_proposal_evidence(store, args.proposal_id).await?;
    evidence.proposal.verify_against(
        &evidence.report,
        evidence.predecessor.as_ref(),
        &evidence.row_reviews,
    )?;
    let mut reviews = store.list_manifest_reviews(args.proposal_id).await?;
    if args.approve {
        let current_row_reviews = store.list_row_reviews(evidence.report.id).await?;
        ensure!(
            evidence.proposal.row_review_references == review_references(&current_row_reviews),
            "proposal {} is stale because newer row reviews exist; rerun `quality curate {}` before approval",
            evidence.proposal.id,
            evidence.report.run_id
        );
        ensure!(
            evidence.proposal.counts.needs_review_rows == 0,
            "proposal {} still has {} rows needing review",
            evidence.proposal.id,
            evidence.proposal.counts.needs_review_rows
        );
        if let Some(review) = reviews.last()
            && review.decision == CurationManifestReviewDecision::Approve
        {
            let manifest = ApprovedCurationManifest::create(
                &evidence.report,
                &evidence.proposal,
                evidence.predecessor.as_ref(),
                &evidence.row_reviews,
                &reviews,
            )?;
            store.save_manifest(&manifest).await?;
            return presentation::print(&serde_json::json!({
                "review": review,
                "manifest": ApprovedManifestSummary::from(&manifest),
            }));
        }
    }
    let decision = if args.approve {
        CurationManifestReviewDecision::Approve
    } else if args.reject {
        CurationManifestReviewDecision::Reject
    } else {
        CurationManifestReviewDecision::RequestRevision
    };
    let review = CurationManifestReview::create(
        &evidence.proposal,
        reviews.last(),
        decision,
        args.reviewer,
        args.reason,
    )?;
    store.append_manifest_review(&review).await?;
    reviews.push(review.clone());
    if decision != CurationManifestReviewDecision::Approve {
        return presentation::print(&review);
    }
    let manifest = ApprovedCurationManifest::create(
        &evidence.report,
        &evidence.proposal,
        evidence.predecessor.as_ref(),
        &evidence.row_reviews,
        &reviews,
    )?;
    store.save_manifest(&manifest).await?;
    presentation::print(&serde_json::json!({
        "review": review,
        "manifest": ApprovedManifestSummary::from(&manifest),
    }))
}

pub(crate) async fn load_verified_manifest(
    store: &SqliteStore,
    manifest_id: Uuid,
) -> anyhow::Result<VerifiedManifestEvidence> {
    let manifest = store
        .get_manifest(manifest_id)
        .await?
        .with_context(|| format!("curation manifest not found: {manifest_id}"))?;
    let proposal = load_proposal_evidence(store, manifest.proposal_id).await?;
    let manifest_reviews = store.list_manifest_reviews(manifest.proposal_id).await?;
    manifest.verify_against(
        &proposal.report,
        &proposal.proposal,
        proposal.predecessor.as_ref(),
        &proposal.row_reviews,
        &manifest_reviews,
    )?;
    Ok(VerifiedManifestEvidence {
        manifest,
        report: proposal.report,
        proposal: proposal.proposal,
        predecessor: proposal.predecessor,
        row_reviews: proposal.row_reviews,
        manifest_reviews,
    })
}

async fn show_manifest(store: &SqliteStore, manifest_id: Uuid) -> anyhow::Result<()> {
    presentation::print(&load_verified_manifest(store, manifest_id).await?.manifest)
}

pub(crate) struct VerifiedManifestEvidence {
    pub manifest: ApprovedCurationManifest,
    pub report: DatasetQualityReport,
    pub proposal: CurationProposal,
    pub predecessor: Option<CurationProposal>,
    pub row_reviews: Vec<RowQualityReview>,
    pub manifest_reviews: Vec<CurationManifestReview>,
}

struct ProposalEvidence {
    proposal: CurationProposal,
    report: DatasetQualityReport,
    predecessor: Option<CurationProposal>,
    row_reviews: Vec<RowQualityReview>,
}

async fn load_proposal_evidence(
    store: &SqliteStore,
    proposal_id: Uuid,
) -> anyhow::Result<ProposalEvidence> {
    let proposal = store
        .get_curation_proposal(proposal_id)
        .await?
        .with_context(|| format!("curation proposal not found: {proposal_id}"))?;
    let stored_report = store
        .get_report(proposal.report_id)
        .await?
        .with_context(|| format!("quality report not found: {}", proposal.report_id))?;
    let report = require_verified_report(store, stored_report.run_id).await?;
    ensure!(
        report == stored_report,
        "curation proposal report differs from its checked run evidence"
    );
    let predecessor = match proposal.predecessor_id {
        Some(id) => Some(
            store
                .get_curation_proposal(id)
                .await?
                .with_context(|| format!("curation proposal predecessor not found: {id}"))?,
        ),
        None => None,
    };
    ensure!(
        predecessor.as_ref().map(|value| value.fingerprint.as_str())
            == proposal.predecessor_fingerprint.as_deref(),
        "curation proposal predecessor fingerprint does not match"
    );
    let all_reviews = store.list_row_reviews(report.id).await?;
    let row_reviews = proposal
        .row_review_references
        .iter()
        .map(|reference| {
            all_reviews
                .iter()
                .find(|review| {
                    review.id == reference.id && review.fingerprint == reference.fingerprint
                })
                .cloned()
                .with_context(|| format!("proposal row review {} is missing", reference.id))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(ProposalEvidence {
        proposal,
        report,
        predecessor,
        row_reviews,
    })
}

fn review_references(reviews: &[RowQualityReview]) -> Vec<ArtifactReference> {
    let mut references = reviews
        .iter()
        .map(|review| ArtifactReference {
            id: review.id,
            fingerprint: review.fingerprint.clone(),
        })
        .collect::<Vec<_>>();
    references.sort();
    references
}

async fn require_run(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<QualityAuditRun> {
    store
        .get_audit_run(run_id)
        .await?
        .with_context(|| format!("quality audit run not found: {run_id}"))
}

async fn require_plan(store: &SqliteStore, plan_id: Uuid) -> anyhow::Result<AuditPlan> {
    store
        .get_audit_plan(plan_id)
        .await?
        .with_context(|| format!("quality audit plan not found: {plan_id}"))
}

async fn require_verified_report(
    store: &SqliteStore,
    run_id: Uuid,
) -> anyhow::Result<DatasetQualityReport> {
    let run = require_run(store, run_id).await?;
    let plan = require_plan(store, run.plan_id).await?;
    store
        .get_audit_guidance(plan.id)
        .await?
        .with_context(|| format!("quality audit plan {} guidance is missing", plan.id))?
        .verify_against(&plan)?;
    let requests = store.list_evaluator_requests(run_id).await?;
    let attempts = store.list_attempts(run_id).await?;
    let assessments = store.list_assessments(run_id).await?;
    run.verify_against_evidence(&plan, &requests, &attempts, &assessments)?;
    let report = store
        .report_for_run(run_id)
        .await?
        .with_context(|| format!("quality audit {run_id} has no completed report"))?;
    report.verify_against(&plan, &run, &requests, &attempts, &assessments)?;
    Ok(report)
}

async fn require_verified_report_by_id(
    store: &SqliteStore,
    report_id: Uuid,
) -> anyhow::Result<DatasetQualityReport> {
    let stored_report = store
        .get_report(report_id)
        .await?
        .with_context(|| format!("quality report not found: {report_id}"))?;
    let report = require_verified_report(store, stored_report.run_id).await?;
    ensure!(
        report == stored_report,
        "quality report {report_id} differs from its fully verified run evidence"
    );
    Ok(report)
}

fn quality_verdict(value: QualityVerdictArg) -> QualityVerdict {
    match value {
        QualityVerdictArg::Qualified => QualityVerdict::Qualified,
        QualityVerdictArg::Borderline => QualityVerdict::Borderline,
        QualityVerdictArg::Quarantined => QualityVerdict::Quarantined,
    }
}

struct EvaluatorBundle {
    primary: EvaluatorIdentity,
    reviewers: Vec<EvaluatorIdentity>,
    adapters: Vec<Arc<dyn QualityEvaluator>>,
}

async fn evaluator_bundle(
    backend: QualityEvaluatorArg,
    independent_reviewers: u8,
    store: &SqliteStore,
) -> anyhow::Result<EvaluatorBundle> {
    match backend {
        QualityEvaluatorArg::Fake => fake_evaluator_bundle(independent_reviewers),
        QualityEvaluatorArg::OpenaiCompatible => {
            openai_compatible_evaluator_bundle(store, independent_reviewers, None).await
        }
    }
}

fn fake_evaluator_bundle(independent_reviewers: u8) -> anyhow::Result<EvaluatorBundle> {
    fake_evaluator_bundle_for_protocol(DEFAULT_EVALUATOR_PROTOCOL, independent_reviewers)
}

fn fake_evaluator_bundle_for_protocol(
    protocol_version: &str,
    independent_reviewers: u8,
) -> anyhow::Result<EvaluatorBundle> {
    let primary = Arc::new(FakeQualityEvaluator::new(
        protocol_version,
        EvaluatorIndependence::Primary,
        42,
    )?);
    let primary_identity = primary.identity();
    let mut reviewers = Vec::with_capacity(usize::from(independent_reviewers));
    let mut adapters: Vec<Arc<dyn QualityEvaluator>> = vec![primary];
    for index in 0..independent_reviewers {
        let reviewer = Arc::new(FakeQualityEvaluator::new(
            protocol_version,
            EvaluatorIndependence::IndependentReview,
            1_001 + u64::from(index),
        )?);
        reviewers.push(reviewer.identity());
        adapters.push(reviewer);
    }
    Ok(EvaluatorBundle {
        primary: primary_identity,
        reviewers,
        adapters,
    })
}

pub(crate) fn fake_evaluator_identities(
    protocol_version: &str,
    independent_reviewers: u8,
) -> anyhow::Result<(EvaluatorIdentity, Vec<EvaluatorIdentity>)> {
    let bundle = fake_evaluator_bundle_for_protocol(protocol_version, independent_reviewers)?;
    Ok((bundle.primary, bundle.reviewers))
}

pub(crate) async fn execute_fake_audit(
    store: &SqliteStore,
    run_id: Uuid,
) -> anyhow::Result<QualityAuditOutcome> {
    let run = require_run(store, run_id).await?;
    ensure!(
        run.primary_evaluator.backend == FAKE_QUALITY_BACKEND
            && run
                .independent_reviewers
                .iter()
                .all(|reviewer| reviewer.backend == FAKE_QUALITY_BACKEND
                    && reviewer.protocol_version == run.primary_evaluator.protocol_version),
        "workflow quality audit is not pinned to the deterministic fake evaluator"
    );
    let reviewer_count = run.independent_reviewers.len().try_into()?;
    let bundle = fake_evaluator_bundle_for_protocol(
        &run.primary_evaluator.protocol_version,
        reviewer_count,
    )?;
    ensure!(
        bundle.primary == run.primary_evaluator && bundle.reviewers == run.independent_reviewers,
        "reconstructed fake evaluator identities do not exactly match the persisted audit run"
    );
    let runner = DatasetQualityRunner::new(
        Arc::new(store.clone()),
        Arc::new(store.clone()),
        bundle.adapters,
    )?;
    Ok(runner.execute(run_id).await?)
}

async fn evaluators_for_run(
    run: &QualityAuditRun,
    store: &SqliteStore,
    api_key_env: &str,
) -> anyhow::Result<Vec<Arc<dyn QualityEvaluator>>> {
    let reviewer_count = run.independent_reviewers.len().try_into()?;
    let bundle = match run.primary_evaluator.backend.as_str() {
        FAKE_QUALITY_BACKEND => fake_evaluator_bundle(reviewer_count)?,
        OPENAI_COMPATIBLE_QUALITY_BACKEND => {
            let api_key = read_optional_api_key(api_key_env)?;
            openai_compatible_evaluator_bundle(store, reviewer_count, api_key).await?
        }
        backend => anyhow::bail!(
            "quality evaluator backend {backend:?} is not available in this CLI build"
        ),
    };
    ensure!(
        bundle.primary == run.primary_evaluator && bundle.reviewers == run.independent_reviewers,
        "reconstructed quality evaluator identities do not exactly match the persisted audit run"
    );
    Ok(bundle.adapters)
}

async fn openai_compatible_evaluator_bundle(
    store: &SqliteStore,
    independent_reviewers: u8,
    api_key: Option<String>,
) -> anyhow::Result<EvaluatorBundle> {
    let backend = store
        .get_backend_configuration(OPENAI_COMPATIBLE_QUALITY_BACKEND)
        .await?
        .context("OpenAI-compatible backend has not been configured")?;
    ensure!(
        backend.name == OPENAI_COMPATIBLE_QUALITY_BACKEND,
        "persisted OpenAI-compatible backend configuration has the wrong identity"
    );
    let base_url = backend
        .base_url
        .context("OpenAI-compatible backend configuration has no base URL")?;

    let primary = Arc::new(OpenAICompatibleQualityEvaluator::new(
        openai_compatible_config(
            &base_url,
            &backend.model,
            EvaluatorIndependence::Primary,
            PRIMARY_EVALUATOR_SEED,
        ),
        api_key.clone(),
    )?);
    let primary_identity = primary.identity();
    let mut reviewers = Vec::with_capacity(usize::from(independent_reviewers));
    let mut adapters: Vec<Arc<dyn QualityEvaluator>> = vec![primary];
    for index in 0..independent_reviewers {
        let seed = REVIEWER_EVALUATOR_SEED_BASE
            .checked_add(i64::from(index))
            .context("quality reviewer seed overflowed")?;
        let reviewer = Arc::new(OpenAICompatibleQualityEvaluator::new(
            openai_compatible_config(
                &base_url,
                &backend.model,
                EvaluatorIndependence::IndependentReview,
                seed,
            ),
            api_key.clone(),
        )?);
        reviewers.push(reviewer.identity());
        adapters.push(reviewer);
    }
    Ok(EvaluatorBundle {
        primary: primary_identity,
        reviewers,
        adapters,
    })
}

pub(crate) async fn openai_compatible_primary_evaluator(
    store: &SqliteStore,
    api_key: Option<String>,
) -> anyhow::Result<Arc<dyn QualityEvaluator>> {
    let mut adapters = openai_compatible_evaluator_bundle(store, 0, api_key)
        .await?
        .adapters;
    adapters
        .pop()
        .context("OpenAI-compatible evaluator bundle has no primary adapter")
}

fn openai_compatible_config(
    base_url: &str,
    model: &str,
    independence: EvaluatorIndependence,
    seed: i64,
) -> OpenAICompatibleEvaluatorConfig {
    let mut config = OpenAICompatibleEvaluatorConfig::new(
        base_url,
        model,
        DEFAULT_EVALUATOR_PROTOCOL,
        independence,
    );
    config.temperature_thousandths = 0;
    config.seed = Some(seed);
    config
}

pub(crate) fn read_optional_api_key(name: &str) -> anyhow::Result<Option<String>> {
    let valid_name = !name.is_empty()
        && name.len() <= 128
        && name.chars().all(|character| {
            character == '_' || character.is_ascii_uppercase() || character.is_ascii_digit()
        })
        && name
            .chars()
            .next()
            .is_some_and(|character| character == '_' || character.is_ascii_uppercase());
    ensure!(
        valid_name,
        "--api-key-env must name an uppercase environment variable, never contain a credential"
    );
    match std::env::var(name) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("the configured evaluator API-key environment value is not valid Unicode")
        }
    }
}

pub(crate) async fn resolve_create_guidance(
    store: &SqliteStore,
    dataset_id: Uuid,
    plan_id: Uuid,
    mode: QualityAuthenticityArg,
) -> anyhow::Result<(GuidanceReferences, EvaluatorGuidance)> {
    let semantic = guidance_from_semantics(
        plan_id,
        super::semantic::resolve_dataset_semantics(store, dataset_id).await?,
    )?;
    let context = if mode == QualityAuthenticityArg::Off {
        None
    } else {
        store.resolve_context(dataset_id).await?
    };
    if mode == QualityAuthenticityArg::Required {
        ensure!(
            context.is_some(),
            "dataset {dataset_id} has no approved authenticity binding"
        );
    }
    let authenticity = context.map(guidance_from_authenticity).transpose()?;
    Ok((
        GuidanceReferences {
            semantic_context: semantic.as_ref().map(|value| value.reference.clone()),
            authenticity_context: authenticity
                .as_ref()
                .map(|(reference, _)| reference.clone()),
        },
        EvaluatorGuidance {
            semantic,
            authenticity: authenticity.map(|(_, guidance)| guidance),
        },
    ))
}

pub(crate) fn guidance_from_semantics(
    plan_id: Uuid,
    context: ResolvedSemanticContext,
) -> anyhow::Result<Option<SemanticEvaluatorGuidance>> {
    ensure!(
        context.reproduce_fingerprint()? == context.fingerprint,
        "resolved semantic context fingerprint does not reproduce"
    );
    if context.targets.is_empty() {
        return Ok(None);
    }
    let mut sources = context
        .sources
        .iter()
        .map(|source| GuidanceReference::new(source.binding_id, source.binding_fingerprint.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    sources.sort();
    sources.dedup();
    ensure!(
        !sources.is_empty(),
        "resolved semantic guidance has no immutable binding sources"
    );
    let labels = context
        .target(&SemanticTarget::Labels)
        .map(semantic_target_guidance);
    let dimensions = context
        .sources
        .iter()
        .filter_map(|source| match &source.target {
            SemanticTarget::Dimension { name } => Some(name.clone()),
            SemanticTarget::Labels => None,
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|name| {
            context
                .target(&SemanticTarget::Dimension { name: name.clone() })
                .map(|target| (name, semantic_target_guidance(target)))
        })
        .collect::<BTreeMap<_, _>>();
    Ok(Some(SemanticEvaluatorGuidance {
        reference: GuidanceReference::new(plan_id, context.fingerprint)?,
        sources,
        labels,
        dimensions,
    }))
}

fn semantic_target_guidance(target: &ResolvedSemanticTarget) -> SemanticTargetGuidance {
    SemanticTargetGuidance {
        description: target.description.clone(),
        entries: target
            .entries
            .iter()
            .map(|(name, entry)| {
                (
                    name.clone(),
                    ConceptGuidance {
                        description: entry.description.clone(),
                        examples: entry.examples.clone(),
                        counterexamples: entry.counterexamples.clone(),
                        inclusion_rules: entry.inclusion_rules.clone(),
                        exclusion_rules: entry.exclusion_rules.clone(),
                    },
                )
            })
            .collect(),
    }
}

pub(crate) fn guidance_from_authenticity(
    context: ResolvedAuthenticityContext,
) -> anyhow::Result<(GuidanceReference, AuthenticityEvaluatorGuidance)> {
    ensure!(
        context.reproduce_fingerprint()? == context.fingerprint,
        "resolved authenticity context failed its integrity check"
    );
    let reference = GuidanceReference::new(context.binding_id, context.binding_fingerprint)?;
    let guidance = AuthenticityEvaluatorGuidance {
        reference: reference.clone(),
        summary: context.summary,
        instructions: context.generation_instructions,
        caveats: context.caveats,
    };
    Ok((reference, guidance))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use generation_core::domain::{BackendConfiguration, GenerationParameters};
    use semantic_catalog::{ResolvedSemanticSource, SemanticEntry, SemanticLayer};

    use super::*;

    fn args(preset: QualityPresetArg) -> QualityPolicyArgs {
        QualityPolicyArgs {
            preset,
            egress: QualityEgressArg::LocalOnly,
            authenticity: QualityAuthenticityArg::Off,
            max_cost_microusd: None,
        }
    }

    #[test]
    fn operator_presets_resolve_to_explicit_full_population_policies() {
        for preset in [
            QualityPresetArg::Fast,
            QualityPresetArg::Balanced,
            QualityPresetArg::Strict,
        ] {
            let policy = compile_policy(&args(preset)).expect("policy");
            assert_eq!(policy.audit_mode, AuditMode::FullPopulation);
            assert_eq!(policy.egress_policy, EvaluatorEgressPolicy::LocalOnly);
            assert!(policy.verify_integrity().is_ok());
        }
    }

    #[test]
    fn fake_evaluator_bundle_has_exact_independent_review_depth() {
        let bundle = fake_evaluator_bundle(2).expect("bundle");
        assert_eq!(bundle.reviewers.len(), 2);
        assert_eq!(bundle.adapters.len(), 3);
        assert_eq!(bundle.primary.independence, EvaluatorIndependence::Primary);
        assert!(
            bundle
                .reviewers
                .iter()
                .all(|reviewer| reviewer.independence == EvaluatorIndependence::IndependentReview)
        );
    }

    #[tokio::test]
    async fn openai_evaluator_bundle_is_key_independent_seeded_and_drift_detecting() {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("temporary store");
        let mut backend = BackendConfiguration {
            name: OPENAI_COMPATIBLE_QUALITY_BACKEND.into(),
            base_url: Some("https://provider.example/v1".into()),
            model: "quality-model".into(),
            parameters: GenerationParameters::default(),
            updated_at: Utc::now(),
        };
        store
            .save_backend_configuration(&backend)
            .await
            .expect("save backend");

        let without_key = openai_compatible_evaluator_bundle(&store, 2, None)
            .await
            .expect("keyless identity construction");
        let with_key = openai_compatible_evaluator_bundle(
            &store,
            2,
            Some("process-only-test-credential".into()),
        )
        .await
        .expect("credentialed identity construction");
        assert_eq!(without_key.primary, with_key.primary);
        assert_eq!(without_key.reviewers, with_key.reviewers);
        assert_eq!(without_key.reviewers.len(), 2);
        assert_ne!(without_key.primary, without_key.reviewers[0]);
        assert_ne!(without_key.reviewers[0], without_key.reviewers[1]);
        assert!(
            std::iter::once(&without_key.primary)
                .chain(&without_key.reviewers)
                .all(|identity| identity.backend == OPENAI_COMPATIBLE_QUALITY_BACKEND
                    && identity.protocol_version == DEFAULT_EVALUATOR_PROTOCOL
                    && identity.execution_location
                        == dataset_quality_core::assessment::EvaluatorExecutionLocation::ExternalService)
        );

        backend.model = "drifted-quality-model".into();
        backend.updated_at = Utc::now();
        store
            .save_backend_configuration(&backend)
            .await
            .expect("save changed backend");
        let drifted = openai_compatible_evaluator_bundle(&store, 2, None)
            .await
            .expect("changed identity construction");
        assert_ne!(without_key.primary, drifted.primary);
        assert_ne!(without_key.reviewers, drifted.reviewers);
    }

    #[test]
    fn quality_api_key_option_accepts_only_environment_variable_names() {
        let error = read_optional_api_key("sk-test-secret-value")
            .expect_err("credential text is not an environment-variable name");
        assert!(!error.to_string().contains("sk-test-secret-value"));
        assert!(
            read_optional_api_key("_ENCODER_GYM_TEST_MISSING_QUALITY_API_KEY")
                .expect("valid missing environment variable")
                .is_none()
        );
    }

    #[test]
    fn semantic_catalog_context_becomes_pinned_evaluator_guidance() {
        let plan_id = Uuid::from_u128(9);
        let binding_id = Uuid::from_u128(10);
        let mut entries = BTreeMap::new();
        entries.insert(
            "billing".into(),
            SemanticEntry {
                description: Some("payment and invoice questions".into()),
                examples: vec!["charged twice".into()],
                ..SemanticEntry::default()
            },
        );
        let mut targets = BTreeMap::new();
        targets.insert(
            SemanticTarget::Labels.key(),
            ResolvedSemanticTarget {
                description: Some("support intent".into()),
                entries,
            },
        );
        let mut context = ResolvedSemanticContext {
            dataset_id: Uuid::from_u128(11),
            targets,
            sources: vec![ResolvedSemanticSource {
                target: SemanticTarget::Labels,
                layer: SemanticLayer::Reusable,
                binding_id,
                binding_fingerprint: "sha256:binding".into(),
                profile_id: Uuid::from_u128(12),
                profile_key: "support-labels".into(),
                profile_version: 1,
                profile_fingerprint: "sha256:profile".into(),
            }],
            resolved_at: Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("time"),
            fingerprint: String::new(),
        };
        context.fingerprint = context
            .reproduce_fingerprint()
            .expect("context fingerprint");

        let guidance = guidance_from_semantics(plan_id, context.clone())
            .expect("guidance conversion")
            .expect("semantic guidance");

        assert_eq!(guidance.reference.id, plan_id);
        assert_eq!(guidance.reference.fingerprint, context.fingerprint);
        assert_eq!(guidance.sources[0].id, binding_id);
        assert_eq!(
            guidance.labels.expect("label guidance").entries["billing"]
                .description
                .as_deref(),
            Some("payment and invoice questions")
        );
    }
}
