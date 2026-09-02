use std::collections::BTreeMap;

use anyhow::Context;
use chrono::Utc;
use encoder_campaign_core::{CampaignEventKind, CampaignStore, replay_campaign};
use encoder_experiment_core::{
    domain::OptimizationBudget,
    metrics::EvaluationReport,
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::{
    collection::DevelopmentObservationRequest,
    diagnosis::{CandidateSuiteOutcome, ComparativeDiagnosis},
    observation::DevelopmentObservationSet,
    ports::{
        DevelopmentObservationBackend, NativeRepairDeltaBackend, NativeRepairQualityStore,
        NativeRepairTrainingStore, RepairEvidenceStore,
    },
    proposal::{
        NativeRepairQualityPolicy, RepairAction, RepairBenchmarkBinding, RepairBudget,
        RepairCandidateHypothesis, RepairContext, RepairProposal, RepairProposalApplication,
        RepairProposalReview, RepairReviewDecision, RepairTarget,
    },
    quality::{
        ApprovedNativeDeltaSelection, NativeDeltaCandidateSet, NativeDeltaQualityReport,
        NativeDeltaReview, NativeDeltaReviewDecision,
    },
    training::NativeRepairTrainingSnapshot,
};
use serde::Deserialize;
use uuid::Uuid;
use workflow_core::{
    benchmark_generation::{BenchmarkGenerationState, replay_benchmark_generation},
    ports::BenchmarkGenerationStore,
};

use crate::{
    cli::{
        NomosWorkspaceArgs, ProductionRepairCampaignArgs, ProductionRepairCommand,
        ProductionRepairDeltaIdArgs, ProductionRepairDeltaReportIdArgs,
        ProductionRepairDeltaReviewArgs, ProductionRepairDeltaReviewDecisionArg,
        ProductionRepairDiagnoseArgs, ProductionRepairIdArgs, ProductionRepairProposalIdArgs,
        ProductionRepairProposeArgs, ProductionRepairReviewArgs, ProductionRepairReviewDecisionArg,
        ProductionRepairSelectionIdArgs, ProductionRepairTrainingSnapshotIdArgs,
    },
    commands::experiment::ensure_database_belongs_to_workspace,
    document, presentation,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepairProposalInput {
    targets: Vec<RepairTargetInput>,
    actions: Vec<RepairAction>,
    quality_policy: NativeRepairQualityPolicyInput,
    budget: RepairBudget,
    candidates: Vec<RepairCandidateHypothesis>,
    expires_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RepairTargetInput {
    key: String,
    slice_fingerprint: String,
    rationale: String,
    absolute_row_target: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRepairQualityPolicyInput {
    protocol_version: String,
    require_task_validation: bool,
    maximum_invalid_rows: u64,
    maximum_exact_duplicates: u64,
    maximum_normalized_duplicates: u64,
    maximum_source_contamination: u64,
    maximum_group_contamination: u64,
    maximum_lineage_contamination: u64,
    require_complete_assessment: bool,
    require_human_approval: bool,
}

pub async fn execute(command: ProductionRepairCommand, database_url: &str) -> anyhow::Result<()> {
    let backend_args = backend_args(&command);
    ensure_database_belongs_to_workspace(database_url, &backend_args.workspace)?;
    let store = SqliteExperimentStore::connect(database_url).await?;
    let backend = NomosBackend::open(&backend_args.workspace, backend_args.python.clone())?;

    match command {
        ProductionRepairCommand::Diagnose(args) => diagnose(&store, &backend, args).await,
        ProductionRepairCommand::Show(args) => show(&store, &backend, args).await,
        ProductionRepairCommand::Doctor(args) => doctor(&store, &backend, args).await,
        ProductionRepairCommand::Evidence(args) => evidence(&store, &backend, args).await,
        ProductionRepairCommand::Propose(args) => propose(&store, &backend, args).await,
        ProductionRepairCommand::ProposalShow(args) => proposal_show(&store, &backend, args).await,
        ProductionRepairCommand::ProposalDoctor(args) => {
            proposal_doctor(&store, &backend, args).await
        }
        ProductionRepairCommand::Review(args) => review(&store, &backend, args).await,
        ProductionRepairCommand::Apply(args) => apply(&store, &backend, args).await,
        ProductionRepairCommand::DeltaBuild(args) => delta_build(&store, &backend, args).await,
        ProductionRepairCommand::DeltaShow(args) => delta_show(&store, args).await,
        ProductionRepairCommand::DeltaDoctor(args) => delta_doctor(&store, &backend, args).await,
        ProductionRepairCommand::DeltaReview(args) => delta_review(&store, &backend, args).await,
        ProductionRepairCommand::DeltaSelect(args) => delta_select(&store, &backend, args).await,
        ProductionRepairCommand::TrainingSnapshotBuild(args) => {
            training_snapshot_build(&store, &backend, args).await
        }
        ProductionRepairCommand::TrainingSnapshotShow(args) => {
            training_snapshot_show(&store, args).await
        }
        ProductionRepairCommand::TrainingSnapshotDoctor(args) => {
            training_snapshot_doctor(&store, &backend, args).await
        }
        ProductionRepairCommand::TrainingExperimentPrepare(args) => {
            training_experiment_prepare(&store, &backend, args).await
        }
    }
}

async fn propose(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairProposeArgs,
) -> anyhow::Result<()> {
    let input: RepairProposalInput = document::read(&args.file)?;
    let diagnosis = load_verified_diagnosis(store, backend, args.diagnosis_id).await?;
    let source_project = store
        .get_project(diagnosis.project_snapshot_id)
        .await?
        .context("repair source project does not exist")?;
    let execution_project = register_current_project(store, backend).await?;
    let benchmark = load_current_benchmark_binding(store, args.benchmark_generation_id).await?;
    let context =
        RepairContext::create(&diagnosis, &source_project, &execution_project, benchmark)?;
    let targets = input
        .targets
        .into_iter()
        .map(|target| {
            let weakness = diagnosis
                .weaknesses
                .iter()
                .find(|value| value.slice_fingerprint == target.slice_fingerprint)
                .with_context(|| {
                    format!(
                        "repair target {} references an unknown diagnosis slice",
                        target.key
                    )
                })?;
            Ok(RepairTarget {
                key: target.key,
                slice_fingerprint: target.slice_fingerprint,
                slice: weakness.slice.clone(),
                weakness_kind: weakness.kind,
                suite_key: weakness.suite_key.clone(),
                rationale: target.rationale,
                absolute_row_target: target.absolute_row_target,
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let quality = input.quality_policy;
    let quality_policy = NativeRepairQualityPolicy::create(
        quality.protocol_version,
        quality.require_task_validation,
        quality.maximum_invalid_rows,
        quality.maximum_exact_duplicates,
        quality.maximum_normalized_duplicates,
        quality.maximum_source_contamination,
        quality.maximum_group_contamination,
        quality.maximum_lineage_contamination,
        quality.require_complete_assessment,
        quality.require_human_approval,
    )?;
    let proposal = RepairProposal::create(
        &diagnosis,
        context,
        targets,
        input.actions,
        quality_policy,
        input.budget,
        input.candidates,
        input.expires_at,
        Utc::now(),
    )?;
    let proposal = store.create_proposal(proposal).await?;
    print_proposal_summary(&proposal, &[])
}

async fn proposal_show(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairProposalIdArgs,
) -> anyhow::Result<()> {
    let proposal = load_verified_proposal(store, backend, args.proposal_id, false).await?;
    let reviews = store.list_proposal_reviews(proposal.id).await?;
    presentation::print(&serde_json::json!({
        "proposal": proposal,
        "reviews": reviews,
        "application": store.get_proposal_application(args.proposal_id).await?,
    }))
}

async fn proposal_doctor(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairProposalIdArgs,
) -> anyhow::Result<()> {
    let proposal = load_verified_proposal(store, backend, args.proposal_id, true).await?;
    let reviews = store.list_proposal_reviews(proposal.id).await?;
    let application = store.get_proposal_application(proposal.id).await?;
    presentation::print(&serde_json::json!({
        "verified": true,
        "current": true,
        "proposal_id": proposal.id,
        "proposal_fingerprint": proposal.fingerprint,
        "specification_fingerprint": proposal.specification_fingerprint,
        "context_fingerprint": proposal.context.fingerprint,
        "diagnosis_id": proposal.context.diagnosis.id,
        "source_project_id": proposal.context.source_project.id,
        "execution_project_id": proposal.context.execution_project.id,
        "benchmark_generation_id": proposal.context.benchmark.generation_id,
        "review_count": reviews.len(),
        "latest_decision": reviews.last().map(|value| value.decision),
        "application_reserved": application.is_some(),
        "expires_at": proposal.expires_at,
    }))
}

async fn review(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairReviewArgs,
) -> anyhow::Result<()> {
    let proposal = load_verified_proposal(store, backend, args.proposal_id, true).await?;
    let diagnosis = store
        .get_diagnosis(proposal.context.diagnosis.id)
        .await?
        .context("repair proposal diagnosis does not exist")?;
    let reviews = store.list_proposal_reviews(proposal.id).await?;
    let decision = match args.decision {
        ProductionRepairReviewDecisionArg::Approve => RepairReviewDecision::Approve,
        ProductionRepairReviewDecisionArg::Reject => RepairReviewDecision::Reject,
        ProductionRepairReviewDecisionArg::RequestRevision => RepairReviewDecision::RequestRevision,
    };
    let review = RepairProposalReview::create(
        &proposal,
        &diagnosis,
        reviews.last(),
        decision,
        args.reviewer,
        args.reason,
        Utc::now(),
    )?;
    let review = store.append_proposal_review(review).await?;
    presentation::print(&review)
}

async fn apply(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairProposalIdArgs,
) -> anyhow::Result<()> {
    let proposal = load_verified_proposal(store, backend, args.proposal_id, true).await?;
    if let Some(existing) = store.get_proposal_application(proposal.id).await? {
        return presentation::print(&existing);
    }
    let diagnosis = store
        .get_diagnosis(proposal.context.diagnosis.id)
        .await?
        .context("repair proposal diagnosis does not exist")?;
    let reviews = store.list_proposal_reviews(proposal.id).await?;
    let approval = reviews
        .last()
        .filter(|value| value.decision == RepairReviewDecision::Approve)
        .context("repair proposal requires a latest exact approval before application")?;
    let approval_index = reviews.len() - 1;
    let application = RepairProposalApplication::reserve(
        &proposal,
        &diagnosis,
        approval,
        approval_index
            .checked_sub(1)
            .and_then(|index| reviews.get(index)),
        Utc::now(),
    )?;
    let application = store.reserve_proposal_application(application).await?;
    presentation::print(&application)
}

struct DeltaApplicationContext {
    project: encoder_experiment_core::domain::ExternalProjectSnapshot,
    diagnosis: ComparativeDiagnosis,
    approval: RepairProposalReview,
    approval_predecessor: Option<RepairProposalReview>,
    application: RepairProposalApplication,
}

async fn load_delta_application_context(
    store: &SqliteExperimentStore,
    proposal: &RepairProposal,
) -> anyhow::Result<DeltaApplicationContext> {
    let project = store
        .get_project(proposal.context.execution_project.id)
        .await?
        .context("repair execution project does not exist")?;
    proposal.context.execution_project.verify(&project)?;
    let diagnosis = store
        .get_diagnosis(proposal.context.diagnosis.id)
        .await?
        .context("repair proposal diagnosis does not exist")?;
    let application = store
        .get_proposal_application(proposal.id)
        .await?
        .context("repair proposal must be applied before native delta construction")?;
    let reviews = store.list_proposal_reviews(proposal.id).await?;
    let approval_index = reviews
        .iter()
        .position(|review| review.id == application.approval_id)
        .context("repair application approval is missing")?;
    if approval_index + 1 != reviews.len() {
        anyhow::bail!("repair application no longer binds the latest proposal review");
    }
    let approval = reviews[approval_index].clone();
    if approval.decision != RepairReviewDecision::Approve {
        anyhow::bail!("repair application does not bind an approval");
    }
    let approval_predecessor = approval_index
        .checked_sub(1)
        .and_then(|index| reviews.get(index))
        .cloned();
    application.validate_against(proposal, &approval)?;
    Ok(DeltaApplicationContext {
        project,
        diagnosis,
        approval,
        approval_predecessor,
        application,
    })
}

async fn build_delta_candidate(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    proposal: &RepairProposal,
) -> anyhow::Result<NativeDeltaCandidateSet> {
    let context = load_delta_application_context(store, proposal).await?;
    backend
        .build_native_delta(
            context.project,
            context.diagnosis,
            proposal.clone(),
            context.approval,
            context.approval_predecessor,
            context.application,
        )
        .await
        .map_err(Into::into)
}

async fn delta_build(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairProposalIdArgs,
) -> anyhow::Result<()> {
    let proposal = load_verified_proposal(store, backend, args.proposal_id, true).await?;
    let candidate_set = build_delta_candidate(store, backend, &proposal).await?;
    let candidate_set = store
        .create_native_delta_candidate_set(candidate_set)
        .await?;
    let report = NativeDeltaQualityReport::create(&proposal, &candidate_set, Utc::now())?;
    let report = store.create_native_delta_report(report).await?;
    print_delta_summary(&candidate_set, &report, &[], None)
}

async fn delta_show(
    store: &SqliteExperimentStore,
    args: ProductionRepairDeltaIdArgs,
) -> anyhow::Result<()> {
    let candidate_set = store
        .get_native_delta_candidate_set(args.candidate_set_id)
        .await?
        .with_context(|| {
            format!(
                "native repair delta candidate set {} does not exist",
                args.candidate_set_id
            )
        })?;
    let report = store
        .get_native_delta_report_for_candidate_set(candidate_set.id)
        .await?
        .context("native repair delta quality report does not exist")?;
    let reviews = store.list_native_delta_reviews(report.id).await?;
    let selection = store
        .get_native_delta_selection_for_proposal(candidate_set.proposal.id)
        .await?;
    presentation::print(&serde_json::json!({
        "candidate_set": candidate_set,
        "quality_report": report,
        "reviews": reviews,
        "selection": selection,
    }))
}

async fn delta_doctor(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairDeltaIdArgs,
) -> anyhow::Result<()> {
    let candidate_set = store
        .get_native_delta_candidate_set(args.candidate_set_id)
        .await?
        .with_context(|| {
            format!(
                "native repair delta candidate set {} does not exist",
                args.candidate_set_id
            )
        })?;
    let proposal = load_verified_proposal(store, backend, candidate_set.proposal.id, true).await?;
    candidate_set.validate_against(&proposal)?;
    let report = store
        .get_native_delta_report_for_candidate_set(candidate_set.id)
        .await?
        .context("native repair delta quality report does not exist")?;
    report.validate_against(&proposal, &candidate_set)?;
    let reviews = store.list_native_delta_reviews(report.id).await?;
    let selection = store
        .get_native_delta_selection_for_proposal(proposal.id)
        .await?;
    let rebuilt = build_delta_candidate(store, backend, &proposal).await?;
    if rebuilt.evidence_fingerprint != candidate_set.evidence_fingerprint {
        anyhow::bail!("native repair delta no longer reproduces its persisted evidence");
    }
    print_delta_doctor(&candidate_set, &report, &reviews, selection.as_ref())
}

async fn delta_review(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairDeltaReviewArgs,
) -> anyhow::Result<()> {
    let report = store
        .get_native_delta_report(args.report_id)
        .await?
        .with_context(|| {
            format!(
                "native repair delta report {} does not exist",
                args.report_id
            )
        })?;
    let candidate_set = store
        .get_native_delta_candidate_set(report.candidate_set.id)
        .await?
        .context("native repair delta candidate set does not exist")?;
    let proposal = load_verified_proposal(store, backend, report.proposal.id, true).await?;
    report.validate_against(&proposal, &candidate_set)?;
    let reviews = store.list_native_delta_reviews(report.id).await?;
    let decision = match args.decision {
        ProductionRepairDeltaReviewDecisionArg::Approve => NativeDeltaReviewDecision::Approve,
        ProductionRepairDeltaReviewDecisionArg::Reject => NativeDeltaReviewDecision::Reject,
        ProductionRepairDeltaReviewDecisionArg::RequestRevision => {
            NativeDeltaReviewDecision::RequestRevision
        }
    };
    let review = NativeDeltaReview::create(
        &proposal,
        &candidate_set,
        &report,
        reviews.last(),
        decision,
        args.reviewer,
        args.reason,
        Utc::now(),
    )?;
    let review = store.append_native_delta_review(review).await?;
    presentation::print(&review)
}

async fn delta_select(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairDeltaReportIdArgs,
) -> anyhow::Result<()> {
    let report = store
        .get_native_delta_report(args.report_id)
        .await?
        .with_context(|| {
            format!(
                "native repair delta report {} does not exist",
                args.report_id
            )
        })?;
    let candidate_set = store
        .get_native_delta_candidate_set(report.candidate_set.id)
        .await?
        .context("native repair delta candidate set does not exist")?;
    let proposal = load_verified_proposal(store, backend, report.proposal.id, true).await?;
    if let Some(existing) = store
        .get_native_delta_selection_for_proposal(proposal.id)
        .await?
    {
        if existing.report.id != report.id {
            anyhow::bail!("repair proposal already has a different approved delta selection");
        }
        return presentation::print(&existing);
    }
    let reviews = store.list_native_delta_reviews(report.id).await?;
    let approval = reviews
        .last()
        .filter(|review| review.decision == NativeDeltaReviewDecision::Approve)
        .context("native repair delta requires a latest exact approval before selection")?;
    let predecessor = reviews
        .len()
        .checked_sub(2)
        .and_then(|index| reviews.get(index));
    let selection = ApprovedNativeDeltaSelection::create(
        &proposal,
        &candidate_set,
        &report,
        approval,
        predecessor,
        Utc::now(),
    )?;
    let selection = store.create_native_delta_selection(selection).await?;
    presentation::print(&selection)
}

struct ApprovedDeltaContext {
    project: encoder_experiment_core::domain::ExternalProjectSnapshot,
    proposal: RepairProposal,
    candidate_set: NativeDeltaCandidateSet,
    report: NativeDeltaQualityReport,
    approval: NativeDeltaReview,
    approval_predecessor: Option<NativeDeltaReview>,
    selection: ApprovedNativeDeltaSelection,
}

async fn load_approved_delta_context(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    selection_id: Uuid,
    require_native_replay: bool,
) -> anyhow::Result<ApprovedDeltaContext> {
    let selection = store
        .get_native_delta_selection(selection_id)
        .await?
        .with_context(|| format!("native repair delta selection {selection_id} does not exist"))?;
    let proposal = load_verified_proposal(store, backend, selection.proposal.id, true).await?;
    let project = store
        .get_project(proposal.context.execution_project.id)
        .await?
        .context("repair execution project does not exist")?;
    proposal.context.execution_project.verify(&project)?;
    let candidate_set = store
        .get_native_delta_candidate_set(selection.candidate_set.id)
        .await?
        .context("native repair delta candidate set does not exist")?;
    let report = store
        .get_native_delta_report(selection.report.id)
        .await?
        .context("native repair delta report does not exist")?;
    let reviews = store.list_native_delta_reviews(report.id).await?;
    let approval_index = reviews
        .iter()
        .position(|value| value.id == selection.approval.id)
        .context("native repair delta selection approval does not exist")?;
    if approval_index + 1 != reviews.len() {
        anyhow::bail!("native repair delta selection does not bind the frozen latest review");
    }
    let approval = reviews[approval_index].clone();
    let approval_predecessor = approval_index
        .checked_sub(1)
        .and_then(|index| reviews.get(index))
        .cloned();
    selection.validate_against(
        &proposal,
        &candidate_set,
        &report,
        &approval,
        approval_predecessor.as_ref(),
    )?;
    if require_native_replay {
        let rebuilt = build_delta_candidate(store, backend, &proposal).await?;
        if rebuilt.evidence_fingerprint != candidate_set.evidence_fingerprint
            || rebuilt.delta_artifact != candidate_set.delta_artifact
        {
            anyhow::bail!("native repair delta no longer reproduces its selected evidence");
        }
    }
    Ok(ApprovedDeltaContext {
        project,
        proposal,
        candidate_set,
        report,
        approval,
        approval_predecessor,
        selection,
    })
}

async fn training_snapshot_build(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairSelectionIdArgs,
) -> anyhow::Result<()> {
    let context = load_approved_delta_context(store, backend, args.selection_id, true).await?;
    if let Some(existing) = store
        .get_native_repair_training_snapshot_for_selection(args.selection_id)
        .await?
    {
        existing.validate_against(
            &context.project,
            &context.proposal,
            &context.candidate_set,
            &context.report,
            &context.approval,
            context.approval_predecessor.as_ref(),
            &context.selection,
        )?;
        return print_training_snapshot_summary(&existing, true);
    }
    let snapshot = NativeRepairTrainingSnapshot::create(
        &context.project,
        &context.proposal,
        &context.candidate_set,
        &context.report,
        &context.approval,
        context.approval_predecessor.as_ref(),
        &context.selection,
        Utc::now(),
    )?;
    let snapshot = store
        .create_native_repair_training_snapshot(snapshot)
        .await?;
    print_training_snapshot_summary(&snapshot, true)
}

async fn training_snapshot_show(
    store: &SqliteExperimentStore,
    args: ProductionRepairTrainingSnapshotIdArgs,
) -> anyhow::Result<()> {
    let snapshot = store
        .get_native_repair_training_snapshot(args.training_snapshot_id)
        .await?
        .with_context(|| {
            format!(
                "native repair training snapshot {} does not exist",
                args.training_snapshot_id
            )
        })?;
    presentation::print(&snapshot)
}

async fn training_snapshot_doctor(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairTrainingSnapshotIdArgs,
) -> anyhow::Result<()> {
    let snapshot = store
        .get_native_repair_training_snapshot(args.training_snapshot_id)
        .await?
        .with_context(|| {
            format!(
                "native repair training snapshot {} does not exist",
                args.training_snapshot_id
            )
        })?;
    let context = load_approved_delta_context(store, backend, snapshot.selection.id, true).await?;
    snapshot.validate_against(
        &context.project,
        &context.proposal,
        &context.candidate_set,
        &context.report,
        &context.approval,
        context.approval_predecessor.as_ref(),
        &context.selection,
    )?;
    print_training_snapshot_summary(&snapshot, true)
}

async fn training_experiment_prepare(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairTrainingSnapshotIdArgs,
) -> anyhow::Result<()> {
    let snapshot = store
        .get_native_repair_training_snapshot(args.training_snapshot_id)
        .await?
        .with_context(|| {
            format!(
                "native repair training snapshot {} does not exist",
                args.training_snapshot_id
            )
        })?;
    let context = load_approved_delta_context(store, backend, snapshot.selection.id, true).await?;
    snapshot.validate_against(
        &context.project,
        &context.proposal,
        &context.candidate_set,
        &context.report,
        &context.approval,
        context.approval_predecessor.as_ref(),
        &context.selection,
    )?;

    let runner = ExperimentRunner::new(store, backend);
    let source_view = runner
        .status(context.proposal.context.source_experiment_run_id)
        .await?;
    let source_protocol = store
        .get_protocol(source_view.protocol_id)
        .await?
        .context("repair source experiment protocol does not exist")?;
    let source_project = store
        .get_project(source_protocol.project_snapshot_id)
        .await?
        .context("repair source experiment project does not exist")?;
    source_protocol.validate_integrity(&source_project)?;
    context
        .proposal
        .context
        .source_project
        .verify(&source_project)?;

    let expected_development = &context
        .proposal
        .context
        .benchmark
        .development_suite_fingerprints;
    let observed_development = source_protocol
        .baseline_development_reports()
        .into_iter()
        .map(|report| (report.suite_key.clone(), report.suite_fingerprint.clone()))
        .collect::<BTreeMap<_, _>>();
    if &observed_development != expected_development
        || source_protocol.baseline_sealed_report.suite_fingerprint
            != context.proposal.context.benchmark.sealed_suite_fingerprint
    {
        anyhow::bail!(
            "repair source protocol does not bind the proposal's exact development and sealed suites"
        );
    }

    let candidates = snapshot.compile_training_candidates(&context.project, &context.proposal)?;
    let budget = OptimizationBudget {
        maximum_candidates: context.proposal.budget.maximum_candidates,
        maximum_training_seconds: context.proposal.budget.maximum_training_seconds,
        maximum_development_evaluations: context.proposal.budget.maximum_development_evaluations,
        maximum_sealed_evaluations: context.proposal.budget.maximum_sealed_uses,
    };
    let protocol = runner
        .prepare_multi_protocol(
            context.project.id,
            source_protocol.metric_contract,
            budget,
            context.proposal.budget.maximum_evaluation_seconds,
            expected_development.keys().cloned().collect(),
            source_protocol.sealed_suite_key,
            candidates,
        )
        .await?;
    crate::commands::experiment::print_prepared_protocol(&protocol)
}

fn print_training_snapshot_summary(
    snapshot: &NativeRepairTrainingSnapshot,
    verified: bool,
) -> anyhow::Result<()> {
    presentation::print(&serde_json::json!({
        "verified": verified,
        "training_snapshot_id": snapshot.id,
        "training_snapshot_fingerprint": snapshot.fingerprint,
        "specification_fingerprint": snapshot.specification_fingerprint,
        "combined_membership_fingerprint": snapshot.combined_membership_fingerprint,
        "proposal_id": snapshot.proposal.id,
        "selection_id": snapshot.selection.id,
        "execution_project_id": snapshot.execution_project.id,
        "baseline_model_fingerprint": snapshot.baseline_model_fingerprint,
        "input_count": snapshot.inputs.len(),
        "inputs": snapshot.inputs,
        "base_rows": snapshot.base_rows,
        "delta_rows": snapshot.delta_rows,
        "total_rows": snapshot.total_rows,
    }))
}

fn print_delta_summary(
    candidate_set: &NativeDeltaCandidateSet,
    report: &NativeDeltaQualityReport,
    reviews: &[NativeDeltaReview],
    selection: Option<&ApprovedNativeDeltaSelection>,
) -> anyhow::Result<()> {
    presentation::print(&serde_json::json!({
        "candidate_set_id": candidate_set.id,
        "candidate_set_fingerprint": candidate_set.fingerprint,
        "evidence_fingerprint": candidate_set.evidence_fingerprint,
        "proposal_id": candidate_set.proposal.id,
        "application_id": candidate_set.application.id,
        "adapter": candidate_set.adapter,
        "delta_artifact": candidate_set.delta_artifact,
        "audit_reference_count": candidate_set.audit_references.len(),
        "candidate_rows": candidate_set.rows.len(),
        "quality_report_id": report.id,
        "quality_report_fingerprint": report.fingerprint,
        "quality_counts": report.counts,
        "eligible": report.eligible,
        "failure_reasons": report.failure_reasons,
        "latest_review": reviews.last(),
        "selection_id": selection.map(|value| value.id),
    }))
}

fn print_delta_doctor(
    candidate_set: &NativeDeltaCandidateSet,
    report: &NativeDeltaQualityReport,
    reviews: &[NativeDeltaReview],
    selection: Option<&ApprovedNativeDeltaSelection>,
) -> anyhow::Result<()> {
    presentation::print(&serde_json::json!({
        "verified": true,
        "native_outputs_reproduced": true,
        "candidate_set_id": candidate_set.id,
        "candidate_set_fingerprint": candidate_set.fingerprint,
        "evidence_fingerprint": candidate_set.evidence_fingerprint,
        "proposal_id": candidate_set.proposal.id,
        "execution_project_id": candidate_set.execution_project_id,
        "delta_artifact": candidate_set.delta_artifact,
        "audit_references": candidate_set.audit_references,
        "candidate_rows": candidate_set.rows.len(),
        "quality_report_id": report.id,
        "quality_report_fingerprint": report.fingerprint,
        "quality_counts": report.counts,
        "eligible": report.eligible,
        "review_count": reviews.len(),
        "latest_review": reviews.last(),
        "selection_id": selection.map(|value| value.id),
        "selection_fingerprint": selection.map(|value| value.fingerprint.as_str()),
    }))
}

async fn register_current_project(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
) -> anyhow::Result<encoder_experiment_core::domain::ExternalProjectSnapshot> {
    let runner = ExperimentRunner::new(store, backend);
    let project = runner.register_project(backend.project_snapshot()?).await?;
    backend.inspect(project.clone()).await?;
    Ok(project)
}

async fn load_current_benchmark_binding(
    store: &SqliteExperimentStore,
    generation_id: Uuid,
) -> anyhow::Result<RepairBenchmarkBinding> {
    let generation = store
        .get_benchmark_generation(generation_id)
        .await?
        .with_context(|| format!("benchmark generation {generation_id} does not exist"))?;
    let events = store
        .list_benchmark_generation_events(generation_id)
        .await?;
    let view = replay_benchmark_generation(&generation, &events)?;
    if view.state != BenchmarkGenerationState::Active {
        anyhow::bail!("repair proposal requires an active benchmark generation");
    }
    let freshness = generation
        .freshness
        .as_ref()
        .context("repair benchmark generation has no freshness authority")?;
    freshness.validate_at(Utc::now())?;
    RepairBenchmarkBinding::create(
        generation.id,
        generation.fingerprint,
        view.last_sequence,
        view.last_event_fingerprint,
        generation
            .development_suites
            .into_iter()
            .map(|value| (value.suite_key, value.bundle.development_suite_fingerprint))
            .collect(),
        generation.sealed_suite_id,
        generation.sealed_suite_fingerprint,
        freshness.valid_until,
    )
    .map_err(Into::into)
}

async fn load_verified_proposal(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    proposal_id: Uuid,
    require_current: bool,
) -> anyhow::Result<RepairProposal> {
    let proposal = store
        .get_proposal(proposal_id)
        .await?
        .with_context(|| format!("production repair proposal {proposal_id} does not exist"))?;
    let diagnosis = store
        .get_diagnosis(proposal.context.diagnosis.id)
        .await?
        .context("repair proposal diagnosis does not exist")?;
    let source = store
        .get_project(proposal.context.source_project.id)
        .await?
        .context("repair source project does not exist")?;
    proposal.context.source_project.verify(&source)?;
    if require_current {
        let execution = register_current_project(store, backend).await?;
        proposal.context.execution_project.verify(&execution)?;
        let benchmark =
            load_current_benchmark_binding(store, proposal.context.benchmark.generation_id).await?;
        let context = RepairContext::create(&diagnosis, &source, &execution, benchmark)?;
        proposal.validate_against(&diagnosis, &context, Utc::now())?;
    } else {
        proposal.validate_integrity(&diagnosis)?;
    }
    Ok(proposal)
}

fn print_proposal_summary(
    proposal: &RepairProposal,
    reviews: &[RepairProposalReview],
) -> anyhow::Result<()> {
    presentation::print(&serde_json::json!({
        "proposal_id": proposal.id,
        "fingerprint": proposal.fingerprint,
        "specification_fingerprint": proposal.specification_fingerprint,
        "diagnosis_id": proposal.context.diagnosis.id,
        "source_project_revision": proposal.context.source_project.source_revision,
        "execution_project_revision": proposal.context.execution_project.source_revision,
        "benchmark_generation_id": proposal.context.benchmark.generation_id,
        "targets": proposal.targets,
        "actions": proposal.actions,
        "candidates": proposal.candidates,
        "budget": proposal.budget,
        "quality_policy_fingerprint": proposal.quality_policy.fingerprint,
        "expires_at": proposal.expires_at,
        "latest_review": reviews.last(),
    }))
}

async fn diagnose(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairDiagnoseArgs,
) -> anyhow::Result<()> {
    if args.minimum_support == 0 || args.maximum_seconds == 0 {
        anyhow::bail!("repair support and collection time limits must be positive");
    }
    let campaign = store
        .get_campaign(args.campaign_id)
        .await?
        .with_context(|| format!("production campaign {} does not exist", args.campaign_id))?;
    let campaign_events = store.list_campaign_events(campaign.id).await?;
    replay_campaign(&campaign, &campaign_events)?;
    let linked_runs = campaign_events
        .iter()
        .filter_map(|event| match event.event {
            CampaignEventKind::RunStarted { run_id, .. } => Some(run_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let run_id = match args.run_id {
        Some(run_id) if linked_runs.contains(&run_id) => run_id,
        Some(_) => anyhow::bail!("requested repair run is not linked to this campaign"),
        None => *linked_runs
            .last()
            .context("campaign has no linked experiment run to diagnose")?,
    };
    let project = store
        .get_project(campaign.project_snapshot_id)
        .await?
        .context("repair campaign project does not exist")?;
    if project.fingerprint != campaign.project_snapshot_fingerprint {
        anyhow::bail!("repair campaign project fingerprint changed");
    }
    backend.inspect(project.clone()).await?;
    let experiment = ExperimentRunner::new(store, backend).status(run_id).await?;
    let protocol = store
        .get_protocol(experiment.protocol_id)
        .await?
        .context("repair source protocol does not exist")?;
    protocol.validate_integrity(&project)?;

    let baseline_reports = protocol
        .baseline_development_reports()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if baseline_reports.len() < 2 {
        anyhow::bail!("comparative repair diagnosis requires at least two development suites");
    }
    let baseline_by_suite = baseline_reports
        .iter()
        .map(|report| (report.suite_key.as_str(), report))
        .collect::<BTreeMap<_, _>>();
    let mut baseline_sets = Vec::with_capacity(baseline_reports.len());
    for report in &baseline_reports {
        baseline_sets.push(
            collect_and_persist(
                store,
                backend,
                &project,
                campaign.id,
                run_id,
                None,
                report,
                &protocol.metric_contract,
                args.dimensions.clone(),
                args.maximum_seconds,
            )
            .await?,
        );
    }

    let mut candidate_sets = Vec::new();
    let mut outcomes = Vec::new();
    for (candidate_id, execution) in &experiment.candidates {
        let train_output = execution.train_output.as_ref().with_context(|| {
            format!("repair candidate {candidate_id} has no immutable trained model")
        })?;
        if execution.development_reports.len() != baseline_by_suite.len()
            || execution.development_assessments.len() != baseline_by_suite.len()
        {
            anyhow::bail!(
                "repair candidate {candidate_id} has incomplete development-suite evidence"
            );
        }
        for (suite_key, baseline) in &baseline_by_suite {
            let report = execution
                .development_reports
                .get(*suite_key)
                .with_context(|| {
                    format!("repair candidate {candidate_id} omitted development suite {suite_key}")
                })?;
            let assessment = execution
                .development_assessments
                .get(*suite_key)
                .with_context(|| {
                    format!("repair candidate {candidate_id} omitted assessment for {suite_key}")
                })?;
            if report.model.fingerprint != train_output.model.fingerprint {
                anyhow::bail!("repair candidate report model identity changed");
            }
            outcomes.push(CandidateSuiteOutcome::create(
                &project,
                &protocol.metric_contract,
                *candidate_id,
                baseline,
                report,
                assessment,
            )?);
            candidate_sets.push(
                collect_and_persist(
                    store,
                    backend,
                    &project,
                    campaign.id,
                    run_id,
                    Some(*candidate_id),
                    report,
                    &protocol.metric_contract,
                    args.dimensions.clone(),
                    args.maximum_seconds,
                )
                .await?,
            );
        }
    }
    let diagnosis = ComparativeDiagnosis::create(
        &project,
        campaign.id,
        run_id,
        args.minimum_support,
        args.dimensions,
        &baseline_sets,
        &candidate_sets,
        &outcomes,
        Utc::now(),
    )?;
    let diagnosis = store.create_diagnosis(diagnosis).await?;
    print_diagnosis_summary(&diagnosis, baseline_sets.len(), candidate_sets.len())
}

#[allow(clippy::too_many_arguments)]
async fn collect_and_persist(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
    campaign_id: Uuid,
    run_id: Uuid,
    candidate_id: Option<Uuid>,
    report: &EvaluationReport,
    contract: &encoder_experiment_core::metrics::MetricContract,
    dimensions: Vec<String>,
    maximum_seconds: u64,
) -> anyhow::Result<DevelopmentObservationSet> {
    let request = DevelopmentObservationRequest::create(
        project,
        report,
        contract,
        dimensions,
        maximum_seconds,
    )?;
    let collected = backend
        .collect_development_observations(project.clone(), request.clone())
        .await?;
    collected.validate_for_request(&request)?;
    let set = DevelopmentObservationSet::create(
        project,
        campaign_id,
        run_id,
        candidate_id,
        report,
        contract,
        collected.observer,
        collected.observation_artifact,
        collected.observations,
        Utc::now(),
    )?;
    store.create_observation_set(set).await.map_err(Into::into)
}

async fn show(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairIdArgs,
) -> anyhow::Result<()> {
    let diagnosis = load_verified_diagnosis(store, backend, args.diagnosis_id).await?;
    presentation::print(&diagnosis)
}

async fn doctor(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairIdArgs,
) -> anyhow::Result<()> {
    let diagnosis = load_verified_diagnosis(store, backend, args.diagnosis_id).await?;
    presentation::print(&serde_json::json!({
        "verified": true,
        "diagnosis_id": diagnosis.id,
        "diagnosis_fingerprint": diagnosis.fingerprint,
        "derivation_fingerprint": diagnosis.derivation_fingerprint,
        "campaign_id": diagnosis.source_campaign_id,
        "experiment_run_id": diagnosis.source_experiment_run_id,
        "observation_set_count": diagnosis.observation_sets.len(),
        "sealed_observation_count": 0,
    }))
}

async fn evidence(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairCampaignArgs,
) -> anyhow::Result<()> {
    let campaign = store
        .get_campaign(args.campaign_id)
        .await?
        .with_context(|| format!("production campaign {} does not exist", args.campaign_id))?;
    let project = store
        .get_project(campaign.project_snapshot_id)
        .await?
        .context("repair campaign project does not exist")?;
    backend.inspect(project).await?;
    let observations = store
        .list_observation_sets_for_campaign(campaign.id)
        .await?;
    let diagnoses = store.list_diagnoses_for_campaign(campaign.id).await?;
    let observation_summaries = observations
        .iter()
        .map(|value| {
            serde_json::json!({
                "id": value.id,
                "evidence_fingerprint": value.evidence_fingerprint,
                "fingerprint": value.fingerprint,
                "experiment_run_id": value.source_experiment_run_id,
                "candidate_id": value.candidate_id,
                "suite_key": value.suite_key,
                "model_fingerprint": value.model_fingerprint,
                "row_count": value.observations.len(),
                "artifact": value.observation_artifact,
            })
        })
        .collect::<Vec<_>>();
    let diagnosis_summaries = diagnoses
        .iter()
        .map(|value| {
            serde_json::json!({
                "id": value.id,
                "derivation_fingerprint": value.derivation_fingerprint,
                "fingerprint": value.fingerprint,
                "experiment_run_id": value.source_experiment_run_id,
                "minimum_support": value.minimum_support,
                "dimensions": value.slice_dimensions,
                "weakness_count": value.weaknesses.len(),
                "candidate_comparison_count": value.candidate_comparisons.len(),
            })
        })
        .collect::<Vec<_>>();
    presentation::print(&serde_json::json!({
        "campaign_id": campaign.id,
        "observation_sets": observation_summaries,
        "diagnoses": diagnosis_summaries,
    }))
}

async fn load_verified_diagnosis(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    diagnosis_id: Uuid,
) -> anyhow::Result<ComparativeDiagnosis> {
    let diagnosis = store
        .get_diagnosis(diagnosis_id)
        .await?
        .with_context(|| format!("production repair diagnosis {diagnosis_id} does not exist"))?;
    let project = store
        .get_project(diagnosis.project_snapshot_id)
        .await?
        .context("repair diagnosis project does not exist")?;
    backend.inspect(project).await?;
    Ok(diagnosis)
}

fn print_diagnosis_summary(
    diagnosis: &ComparativeDiagnosis,
    baseline_set_count: usize,
    candidate_set_count: usize,
) -> anyhow::Result<()> {
    let tradeoffs = diagnosis
        .candidate_tradeoffs
        .iter()
        .map(|value| {
            serde_json::json!({
                "candidate_id": value.candidate_id,
                "model_fingerprint": value.model_fingerprint,
                "passed_suites": value.passed_suites,
                "failed_suites": value.failed_suites,
            })
        })
        .collect::<Vec<_>>();
    presentation::print(&serde_json::json!({
        "diagnosis_id": diagnosis.id,
        "fingerprint": diagnosis.fingerprint,
        "derivation_fingerprint": diagnosis.derivation_fingerprint,
        "campaign_id": diagnosis.source_campaign_id,
        "experiment_run_id": diagnosis.source_experiment_run_id,
        "baseline_observation_sets": baseline_set_count,
        "candidate_observation_sets": candidate_set_count,
        "weaknesses": diagnosis.weaknesses.len(),
        "candidate_comparisons": diagnosis.candidate_comparisons.len(),
        "candidate_tradeoffs": tradeoffs,
        "sealed_observation_count": 0,
    }))
}

fn backend_args(command: &ProductionRepairCommand) -> &NomosWorkspaceArgs {
    match command {
        ProductionRepairCommand::Diagnose(args) => &args.backend,
        ProductionRepairCommand::Show(args) | ProductionRepairCommand::Doctor(args) => {
            &args.backend
        }
        ProductionRepairCommand::Evidence(args) => &args.backend,
        ProductionRepairCommand::Propose(args) => &args.backend,
        ProductionRepairCommand::ProposalShow(args)
        | ProductionRepairCommand::ProposalDoctor(args)
        | ProductionRepairCommand::Apply(args)
        | ProductionRepairCommand::DeltaBuild(args) => &args.backend,
        ProductionRepairCommand::Review(args) => &args.backend,
        ProductionRepairCommand::DeltaShow(args) | ProductionRepairCommand::DeltaDoctor(args) => {
            &args.backend
        }
        ProductionRepairCommand::DeltaReview(args) => &args.backend,
        ProductionRepairCommand::DeltaSelect(args) => &args.backend,
        ProductionRepairCommand::TrainingSnapshotBuild(args) => &args.backend,
        ProductionRepairCommand::TrainingSnapshotShow(args)
        | ProductionRepairCommand::TrainingSnapshotDoctor(args)
        | ProductionRepairCommand::TrainingExperimentPrepare(args) => &args.backend,
    }
}
