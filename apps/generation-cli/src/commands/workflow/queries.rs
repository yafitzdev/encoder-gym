use super::*;

pub(super) async fn print_status(store: &SqliteStore, run: WorkflowRun) -> anyhow::Result<()> {
    let attempts = store.list_workflow_attempts(run.id).await?;
    let mut child_executions = Vec::new();
    for attempt in &attempts {
        child_executions.extend(store.list_workflow_child_executions(attempt.id).await?);
    }
    let mut generation = Vec::new();
    for plan_id in attempts
        .iter()
        .flat_map(|attempt| &attempt.artifacts)
        .filter(|artifact| {
            artifact.kind == "generation_plan" || artifact.kind == "iteration_generation_plan"
        })
        .map(|artifact| artifact.artifact_id)
    {
        let Some(plan) = store.get_plan(plan_id).await? else {
            continue;
        };
        let jobs = store
            .list_jobs(JobQuery {
                plan_id: Some(plan_id),
                limit: 10_000,
                ..JobQuery::default()
            })
            .await?;
        let coverage =
            calculate_coverage(&plan, &store.dataset_cell_counts(plan.dataset_id).await?);
        generation.push(serde_json::json!({
            "plan_id": plan_id,
            "jobs": jobs,
            "coverage": coverage,
        }));
    }
    let definition = require_definition(store, run.definition_id).await?;
    let mut evidence_risk = Vec::new();
    for suite_id in
        std::iter::once(definition.development_suite_id).chain(definition.sealed_suite_id)
    {
        let Some(suite) = store.get_benchmark_suite(suite_id).await? else {
            continue;
        };
        for cohort in &suite.cohorts {
            let exposures = store
                .query_exposures(ExposureQuery {
                    cohort_id: cohort.cohort_id,
                    purpose: None,
                    limit: 10_000,
                    offset: 0,
                })
                .await?;
            evidence_risk.push(serde_json::json!({
                "suite_id": suite.id,
                "suite_kind": suite.kind,
                "cohort_id": cohort.cohort_id,
                "risk": summarize_exposure_risk(cohort.cohort_id, &exposures)?,
            }));
        }
    }
    crate::presentation::print(&serde_json::json!({
        "run": run,
        "attempt_count": attempts.len(),
        "latest_attempt": attempts.last(),
        "attempts": attempts,
        "child_executions": child_executions,
        "generation": generation,
        "evidence_risk": evidence_risk,
    }))
}

pub(super) async fn resolve_pending_workflow_recovery(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<()> {
    if store
        .list_recovery_records(false)
        .await?
        .iter()
        .any(|record| {
            record.workflow_kind == WorkflowKind::EncoderWorkflow && record.workflow_id == id
        })
    {
        store
            .resolve_recovery(
                WorkflowKind::EncoderWorkflow,
                id,
                RecoveryState::Resumed,
                None,
            )
            .await?;
    }
    Ok(())
}

pub(super) async fn require_definition(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<WorkflowDefinition> {
    store
        .get_workflow_definition(id)
        .await?
        .with_context(|| format!("workflow definition not found: {id}"))
}

pub(super) async fn require_run(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<WorkflowRun> {
    store
        .get_workflow_run(id)
        .await?
        .with_context(|| format!("workflow run not found: {id}"))
}
