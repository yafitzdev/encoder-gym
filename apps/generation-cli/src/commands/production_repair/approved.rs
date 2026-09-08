//! Verification of frozen repair evidence using persisted facts only.
use anyhow::Context;
use encoder_experiment_core::{domain::ExternalProjectSnapshot, ports::ExperimentStore};
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::{
    ports::{NativeRepairQualityStore, RepairEvidenceStore},
    proposal::RepairProposal,
    quality::{
        ApprovedNativeDeltaSelection, NativeDeltaCandidateSet, NativeDeltaQualityReport,
        NativeDeltaReview,
    },
};
use uuid::Uuid;

pub(crate) struct ApprovedDeltaContext {
    pub(crate) project: ExternalProjectSnapshot,
    pub(crate) proposal: RepairProposal,
    pub(crate) candidate_set: NativeDeltaCandidateSet,
    pub(crate) report: NativeDeltaQualityReport,
    pub(crate) approval: NativeDeltaReview,
    pub(crate) approval_predecessor: Option<NativeDeltaReview>,
    pub(crate) selection: ApprovedNativeDeltaSelection,
}

pub(crate) async fn load_approved_delta_facts(
    store: &SqliteExperimentStore,
    selection_id: Uuid,
) -> anyhow::Result<ApprovedDeltaContext> {
    let selection = store
        .get_native_delta_selection(selection_id)
        .await?
        .with_context(|| format!("native repair delta selection {selection_id} does not exist"))?;
    let proposal = load_frozen_proposal(store, selection.proposal.id).await?;
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

pub(super) async fn load_frozen_proposal(
    store: &SqliteExperimentStore,
    proposal_id: Uuid,
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
    proposal.validate_integrity(&diagnosis)?;
    Ok(proposal)
}
