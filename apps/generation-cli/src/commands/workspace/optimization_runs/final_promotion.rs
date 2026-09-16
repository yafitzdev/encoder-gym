//! Explicit manual handoff to the ordinary baseline-revision contract.
use anyhow::{Context, Result, ensure};
use encoder_experiment_core::{domain::ModelArtifactIdentity, ports::EncoderTaskBackend};
use encoder_experiment_nomos::NomosBackend;
use project_workspace_core::{
    BaselineChange, BoundIdentity, ModelArtifact,
    optimization_final_promotion::{AgentFinalPromotionEvidence, agent_final_promotion_run},
};
use project_workspace_local::{
    AcceptedModelPromotion, ManagedWorkspace, open_workspace,
    optimization_final_execution::{self as custody, AgentFinalExecution},
    optimization_iteration_execution, record_accepted_model_promotion,
};
use std::path::Path;
use uuid::Uuid;

struct AcceptedFinalCandidate {
    backend: NomosBackend,
    native_model: ModelArtifactIdentity,
    model: ModelArtifact,
    execution: AgentFinalExecution,
    decision: BoundIdentity,
}

async fn verified(folder: &Path, run_id: Uuid) -> Result<AcceptedFinalCandidate> {
    let scientific = super::final_authorization::history(folder, run_id).await?;
    let execution = custody::show(folder, run_id, &scientific)
        .await?
        .context("Separate final consent is missing")?;
    let receipt = execution
        .result
        .as_ref()
        .context("Complete final acceptance before promotion")?;
    let dispatch = execution
        .dispatch
        .as_ref()
        .context("Final dispatch is missing")?;
    let source = super::final_execution::selected(&execution, &scientific)?;
    let (backend, native_model) =
        super::final_execution::backend(folder, run_id, &execution, &scientific).await?;
    let report = backend
        .recover_evaluation(
            source.project.clone(),
            native_model.clone(),
            source.protocol.metric_contract.clone(),
            execution.authorization.scope.final_suite.clone(),
        )
        .await?
        .context("Completed final evidence is missing; it will not be evaluated again")?;
    let result = receipt.recover(
        &execution.authorization,
        dispatch,
        &source.project,
        &source.protocol,
        report,
    )?;
    let scope = &execution.authorization.scope;
    let training = optimization_iteration_execution::training(folder, run_id, source.iteration_id)
        .await?
        .context("Selected training custody is missing")?;
    let workspace = open_workspace(folder, true).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model catalog is missing")?;
    let models: Vec<_> = catalog
        .artifacts
        .iter()
        .filter(|model| {
            model.source_model.as_ref() == Some(&scope.model)
                && model
                    .producing_run
                    .as_ref()
                    .is_some_and(|run| run.id == training.experiment_run_id.to_string())
        })
        .collect();
    ensure!(
        models.len() == 1,
        "Selected final checkpoint must have exactly one registered model"
    );
    let model = models[0].clone();
    let baseline = catalog
        .baseline_revisions
        .iter()
        .find(|revision| revision.id.to_string() == scope.comparison_baseline_revision.id)
        .context("Original comparison baseline revision is missing")?;
    let decision = AgentFinalPromotionEvidence {
        authorization: &execution.authorization,
        dispatch,
        result: &result,
        training: &training,
        project: &source.project,
        protocol: &source.protocol,
        events: &source.events,
        model: &model,
        baseline,
    }
    .decision()?;
    Ok(AcceptedFinalCandidate {
        backend,
        native_model,
        model,
        execution,
        decision,
    })
}

pub(super) async fn execute(
    folder: &Path,
    run_id: Uuid,
    expected_baseline_revision: Uuid,
    expected_final_receipt: &str,
    actor: String,
    reason: String,
) -> Result<()> {
    let project_url = super::super::sqlite_file_url(&folder.join("project.sqlite"));
    let _lease = crate::commands::encoder_optimize::OptimizationExecutionLease::acquire(
        &project_url,
        run_id,
    )
    .await?;
    let accepted = verified(folder, run_id).await?;
    ensure!(
        accepted
            .execution
            .authorization
            .scope
            .comparison_baseline_revision
            .id
            == expected_baseline_revision.to_string()
            && accepted.decision.fingerprint == expected_final_receipt,
        "Final receipt or original baseline differs from the reviewed promotion"
    );
    let workspace = open_workspace(folder, true).await?;
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model catalog missing")?;
    if let Some(existing) = catalog.baseline_revisions.iter().find(|revision| {
        matches!(&revision.change,
        BaselineChange::Promotion { decision_id, .. } if decision_id == &accepted.decision.id)
    }) {
        ensure!(
            existing.actor == actor
                && existing.reason == reason
                && existing.previous_revision_id == Some(expected_baseline_revision)
                && existing.model_artifact_id == accepted.model.id,
            "Promotion retry changed its original request"
        );
    }
    let source = accepted
        .backend
        .verified_model_path(&accepted.native_model)?;
    let model = accepted.model;
    let promoted = record_accepted_model_promotion(
        folder,
        &source,
        AcceptedModelPromotion {
            expected_baseline_revision_id: expected_baseline_revision,
            name: model.name,
            source_model: model.source_model.context("Source model missing")?,
            source_model_format: model.format,
            source_model_bytes: model.bytes,
            producing_run: model.producing_run.context("Producing run missing")?,
            training_snapshot: model
                .training_snapshot
                .context("Training snapshot missing")?,
            trainer: model.trainer.context("Trainer missing")?,
            effective_configuration_fingerprint: model
                .effective_configuration_fingerprint
                .context("Training settings missing")?,
            source_revision: model.source_revision.context("Source revision missing")?,
            decision_id: accepted.decision.id,
            decision_fingerprint: accepted.decision.fingerprint,
            actor,
            reason,
        },
    )
    .await?;
    super::super::print(&promoted)
}

/// Resolve only the promotion linked to this managed model; no sibling-run scan.
/// Restoring a previously promoted model keeps that original decision authority.
pub(in crate::commands::workspace) async fn resolve_promoted(
    workspace: &ManagedWorkspace,
    model: &ModelArtifact,
) -> Result<Option<ModelArtifactIdentity>> {
    let catalog = workspace
        .model_catalog
        .as_ref()
        .context("Model catalog missing")?;
    let mut selected = None;
    for revision in catalog
        .baseline_revisions
        .iter()
        .filter(|revision| revision.model_artifact_id == model.id)
    {
        let BaselineChange::Promotion {
            decision_id,
            decision_fingerprint,
        } = &revision.change
        else {
            continue;
        };
        let Some(run_id) = agent_final_promotion_run(decision_id)? else {
            continue;
        };
        ensure!(
            selected.is_none(),
            "Model has ambiguous Agent final promotion authority"
        );
        let accepted = verified(Path::new(&workspace.folder), run_id).await?;
        ensure!(
            &accepted.model == model
                && accepted.decision.id == *decision_id
                && accepted.decision.fingerprint == *decision_fingerprint
                && revision
                    .previous_revision_id
                    .map(|id| id.to_string())
                    .as_ref()
                    == Some(
                        &accepted
                            .execution
                            .authorization
                            .scope
                            .comparison_baseline_revision
                            .id
                    ),
            "Promoted Agent checkpoint differs from its original final decision"
        );
        selected = Some(accepted.native_model);
    }
    Ok(selected)
}
