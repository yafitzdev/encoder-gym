use anyhow::{Context, ensure};
use dataset_quality_core::{
    curation::CurationProposal,
    lifecycle::{QualityAuditRun, QualityAuditRunState},
    population::AuditPlan,
    ports::{DatasetQualityStore, QualityCandidateSource},
};
use dataset_quality_fake::FAKE_QUALITY_BACKEND;
use generation_core::ports::DatasetStore;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;
use workflow_core::execution::WorkflowChildKind;
use workflow_core::workflow::{
    WORKFLOW_QUALITY_EVALUATOR_BACKEND, WorkflowArtifactLink, WorkflowDefinition,
    WorkflowQualityAuthenticity, WorkflowRun, WorkflowStage, WorkflowStageAttempt,
};

use super::{artifact_id, artifact_link, link};
use crate::{
    cli::QualityAuthenticityArg,
    commands::quality::{self, VerifiedManifestEvidence},
};

pub(super) struct AuditArtifacts {
    pub(super) links: Vec<WorkflowArtifactLink>,
}

pub(super) struct CurationGateStatus {
    pub(super) proposal: CurationProposal,
    pub(super) approved: Option<VerifiedManifestEvidence>,
}

pub(super) async fn execute_audit(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    attempt: &WorkflowStageAttempt,
) -> anyhow::Result<AuditArtifacts> {
    let stage = attempt.stage;
    ensure!(
        matches!(
            stage,
            WorkflowStage::QualityAudit | WorkflowStage::IterationQualityAudit
        ),
        "quality audit helper received a non-audit workflow stage"
    );
    let gate = definition
        .quality_gate
        .as_ref()
        .context("workflow quality stage has no immutable gate")?;
    ensure!(
        gate.evaluator_backend == WORKFLOW_QUALITY_EVALUATOR_BACKEND
            && FAKE_QUALITY_BACKEND == WORKFLOW_QUALITY_EVALUATOR_BACKEND,
        "workflow quality evaluator backend {:?} does not match the available unattended adapter {:?}",
        gate.evaluator_backend,
        WORKFLOW_QUALITY_EVALUATOR_BACKEND,
    );
    ensure!(
        !gate.evaluator_protocol_version.trim().is_empty(),
        "workflow quality evaluator protocol version is empty"
    );

    let plan_id = audit_plan_id(definition, run, stage)?;
    let audit_run_id = audit_run_id(definition, run, stage)?;
    let dataset = store
        .get_dataset(definition.dataset_id)
        .await?
        .context("workflow quality dataset not found")?;
    let source_rows = store.list_source_rows(definition.dataset_id).await?;

    let (plan, mut audit_run) = match store.get_audit_plan(plan_id).await? {
        Some(stored_plan) => {
            let guidance = store
                .get_audit_guidance(plan_id)
                .await?
                .context("workflow quality audit guidance is missing")?;
            guidance.verify_against(&stored_plan)?;
            let expected_plan = AuditPlan::with_identity(
                plan_id,
                &dataset,
                gate.policy.clone(),
                stored_plan.guidance.clone(),
                guidance.reproduce_fingerprint()?,
                gate.evaluator_protocol_version.clone(),
                source_rows,
                stored_plan.created_at,
            )?;
            ensure!(
                expected_plan == stored_plan,
                "persisted workflow quality plan no longer matches the exact gate, source population, or guidance"
            );
            let stored_run = store
                .get_audit_run(audit_run_id)
                .await?
                .context("workflow quality audit run is missing")?;
            let expected_run = quality_run(
                audit_run_id,
                run.created_at,
                &stored_plan,
                &gate.evaluator_protocol_version,
            )?;
            ensure!(
                stored_run.specification_fingerprint == expected_run.specification_fingerprint
                    && stored_run.id == expected_run.id
                    && stored_run.plan_id == expected_run.plan_id
                    && stored_run.primary_evaluator == expected_run.primary_evaluator
                    && stored_run.independent_reviewers == expected_run.independent_reviewers,
                "persisted workflow quality run does not match the exact immutable gate"
            );
            stored_run.verify_integrity(&stored_plan)?;
            (stored_plan, stored_run)
        }
        None => {
            ensure!(
                store.get_audit_run(audit_run_id).await?.is_none(),
                "workflow quality run exists without its deterministic plan"
            );
            let authenticity = match gate.authenticity {
                WorkflowQualityAuthenticity::Off => QualityAuthenticityArg::Off,
                WorkflowQualityAuthenticity::Required => QualityAuthenticityArg::Required,
            };
            let (guidance_references, guidance) = quality::resolve_create_guidance(
                store,
                definition.dataset_id,
                plan_id,
                authenticity,
            )
            .await?;
            let plan = AuditPlan::with_identity(
                plan_id,
                &dataset,
                gate.policy.clone(),
                guidance_references,
                guidance.reproduce_fingerprint()?,
                gate.evaluator_protocol_version.clone(),
                source_rows,
                run.created_at,
            )?;
            let audit_run = quality_run(
                audit_run_id,
                run.created_at,
                &plan,
                &gate.evaluator_protocol_version,
            )?;
            store.create_audit(&plan, &audit_run, &guidance).await?;
            (plan, audit_run)
        }
    };

    ensure!(
        audit_run.primary_evaluator.backend == gate.evaluator_backend
            && audit_run.primary_evaluator.protocol_version == gate.evaluator_protocol_version
            && audit_run
                .independent_reviewers
                .iter()
                .all(|identity| identity.backend == gate.evaluator_backend
                    && identity.protocol_version == gate.evaluator_protocol_version),
        "workflow quality evaluator identity differs from the immutable gate"
    );
    let outcome = match audit_run.state {
        QualityAuditRunState::Queued | QualityAuditRunState::Running => {
            let child = super::child_execution::reserve(
                store,
                attempt,
                WorkflowChildKind::QualityAuditRun,
                "primary",
                audit_run.id,
            )
            .await?;
            super::child_execution::synchronize_parent_before_start(store, run.id, &child).await?;
            quality::execute_fake_audit(store, audit_run.id).await?
        }
        QualityAuditRunState::Completed => quality::execute_fake_audit(store, audit_run.id).await?,
        QualityAuditRunState::Failed | QualityAuditRunState::Cancelled => {
            anyhow::bail!(
                "workflow quality audit {} is terminal in state {:?}",
                audit_run.id,
                audit_run.state
            )
        }
    };
    audit_run = outcome.run;
    ensure!(
        audit_run.state == QualityAuditRunState::Completed,
        "workflow quality audit did not complete"
    );
    audit_run.verify_integrity(&plan)?;
    let report = outcome
        .report
        .context("completed workflow quality audit has no report")?;
    report.verify_integrity()?;
    ensure!(
        report.run_id == audit_run.id
            && report.plan_id == plan.id
            && report.source_set_fingerprint == plan.source_set_fingerprint,
        "workflow quality report does not match its pinned audit"
    );

    let (plan_kind, run_kind, report_kind) = audit_artifact_kinds(stage)?;
    Ok(AuditArtifacts {
        links: vec![
            link(plan_kind, plan.id, &plan.fingerprint),
            link(run_kind, audit_run.id, &audit_run.specification_fingerprint),
            link(report_kind, report.id, &report.fingerprint),
        ],
    })
}

pub(super) async fn create_review_proposal(
    store: &SqliteStore,
    history: &[WorkflowStageAttempt],
    stage: WorkflowStage,
) -> anyhow::Result<CurationProposal> {
    let (run_kind, report_kind, _) = review_artifact_kinds(stage)?;
    let audit_run_id = artifact_id(history, run_kind)?;
    let report_link = artifact_link(history, report_kind)?;
    let proposal = quality::create_or_reuse_curation_proposal(store, audit_run_id).await?;
    let report = store
        .get_report(proposal.report_id)
        .await?
        .context("workflow quality report not found")?;
    ensure!(
        report.id == report_link.artifact_id
            && report.fingerprint == report_link.artifact_fingerprint
            && proposal.report_id == report.id
            && proposal.report_fingerprint == report.fingerprint,
        "workflow curation proposal does not match the exact audited report"
    );
    Ok(proposal)
}

/// Returns an approved manifest only when it seals the current latest proposal
/// for the exact report linked by this workflow cycle. An approval for a stale
/// predecessor never resumes the workflow.
pub(super) async fn approved_manifest(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    history: &[WorkflowStageAttempt],
    review_stage: WorkflowStage,
) -> anyhow::Result<CurationGateStatus> {
    let (audit_run_kind, report_kind, _) = review_artifact_kinds(review_stage)?;
    let report_link = artifact_link(history, report_kind)?;
    let audit_run_id = artifact_id(history, audit_run_kind)?;
    // Materialize an immutable successor whenever append-only row reviews have
    // changed since the proposal shown at the pause. This makes an approval of
    // the stale predecessor ineligible to resume the workflow.
    let proposal = quality::create_or_reuse_curation_proposal(store, audit_run_id).await?;
    ensure!(
        proposal.report_id == report_link.artifact_id
            && proposal.report_fingerprint == report_link.artifact_fingerprint,
        "latest workflow curation proposal belongs to different report evidence"
    );
    let Some(manifest) = store.manifest_for_proposal(proposal.id).await? else {
        return Ok(CurationGateStatus {
            proposal,
            approved: None,
        });
    };
    let evidence = quality::load_verified_manifest(store, manifest.id).await?;
    ensure!(
        evidence.report.id == report_link.artifact_id
            && evidence.report.fingerprint == report_link.artifact_fingerprint
            && evidence.proposal == proposal
            && evidence.manifest.proposal_id == proposal.id
            && evidence.manifest.proposal_fingerprint == proposal.fingerprint
            && evidence.manifest.dataset_definition_id == definition.dataset_id,
        "approved curation manifest does not seal the exact latest workflow proposal"
    );
    Ok(CurationGateStatus {
        proposal,
        approved: Some(evidence),
    })
}

pub(super) fn audit_run_id(
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    stage: WorkflowStage,
) -> anyhow::Result<Uuid> {
    stable_artifact_uuid(&serde_json::json!({
        "kind": "workflow_quality_audit_run",
        "workflow_definition_fingerprint": definition.fingerprint,
        "workflow_run_id": run.id,
        "iteration": run.iteration,
        "stage": stage,
    }))
}

fn audit_plan_id(
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    stage: WorkflowStage,
) -> anyhow::Result<Uuid> {
    stable_artifact_uuid(&serde_json::json!({
        "kind": "workflow_quality_audit_plan",
        "workflow_definition_fingerprint": definition.fingerprint,
        "workflow_run_id": run.id,
        "iteration": run.iteration,
        "stage": stage,
    }))
}

fn stable_artifact_uuid(value: &impl serde::Serialize) -> anyhow::Result<Uuid> {
    let fingerprint = artifact_core::fingerprint(value)?;
    let hex = fingerprint
        .strip_prefix("sha256:")
        .context("artifact fingerprint has no sha256 prefix")?;
    ensure!(hex.len() >= 32, "artifact fingerprint is too short");
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let start = index * 2;
        *byte = u8::from_str_radix(&hex[start..start + 2], 16)
            .context("artifact fingerprint is not hexadecimal")?;
    }
    // RFC 9562 version 8 is reserved for application-defined deterministic
    // UUIDs. Preserve the RFC variant bits while deriving the payload from the
    // canonical artifact fingerprint.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Ok(Uuid::from_bytes(bytes))
}

fn quality_run(
    id: Uuid,
    created_at: chrono::DateTime<chrono::Utc>,
    plan: &AuditPlan,
    protocol_version: &str,
) -> anyhow::Result<QualityAuditRun> {
    let (primary, reviewers) = quality::fake_evaluator_identities(
        protocol_version,
        plan.policy
            .borderline_review_policy
            .required_additional_assessments(),
    )?;
    let mut run = QualityAuditRun::queue(plan, primary, reviewers)?;
    run.id = id;
    run.created_at = created_at;
    run.specification_fingerprint = run.reproduce_specification_fingerprint()?;
    run.verify_integrity(plan)?;
    Ok(run)
}

fn audit_artifact_kinds(
    stage: WorkflowStage,
) -> anyhow::Result<(&'static str, &'static str, &'static str)> {
    match stage {
        WorkflowStage::QualityAudit => {
            Ok(("quality_audit_plan", "quality_audit_run", "quality_report"))
        }
        WorkflowStage::IterationQualityAudit => Ok((
            "iteration_quality_audit_plan",
            "iteration_quality_audit_run",
            "iteration_quality_report",
        )),
        _ => anyhow::bail!("workflow stage is not a quality audit"),
    }
}

fn review_artifact_kinds(
    stage: WorkflowStage,
) -> anyhow::Result<(&'static str, &'static str, &'static str)> {
    match stage {
        WorkflowStage::CurationReview | WorkflowStage::CurationApproval => {
            Ok(("quality_audit_run", "quality_report", "curation_proposal"))
        }
        WorkflowStage::IterationCurationReview | WorkflowStage::IterationCurationApproval => Ok((
            "iteration_quality_audit_run",
            "iteration_quality_report",
            "iteration_curation_proposal",
        )),
        _ => anyhow::bail!("workflow stage is not a curation review"),
    }
}

pub(super) fn proposal_kind(stage: WorkflowStage) -> anyhow::Result<&'static str> {
    review_artifact_kinds(stage).map(|(_, _, proposal)| proposal)
}

#[cfg(test)]
mod tests {
    use super::stable_artifact_uuid;

    #[test]
    fn workflow_quality_artifact_ids_are_stable_and_domain_separated() {
        let first = stable_artifact_uuid(&serde_json::json!({"kind": "plan", "run": 1}))
            .expect("stable UUID");
        let replay = stable_artifact_uuid(&serde_json::json!({"kind": "plan", "run": 1}))
            .expect("stable UUID replay");
        let other = stable_artifact_uuid(&serde_json::json!({"kind": "run", "run": 1}))
            .expect("separate UUID");
        assert_eq!(first, replay);
        assert_ne!(first, other);
        assert_eq!(first.get_version_num(), 8);
    }
}
