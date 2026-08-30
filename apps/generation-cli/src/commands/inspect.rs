use anyhow::Context;
use generation_core::{
    coverage::calculate_coverage,
    domain::ValidationStatus,
    jobs::JobState,
    ports::{
        DatasetStore, GenerationExecutionStore, JobQuery, JobStore, PlanStore, RowQuery, RowStore,
    },
    prompting::PromptBuilder,
};
use semantic_catalog::SemanticCatalogStore;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::cli::{JobCommand, JobStateArg, RowStatus, RowsArgs};

pub async fn job(command: JobCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        JobCommand::List {
            dataset_id,
            plan_id,
            state,
            page,
        } => {
            let jobs = store
                .list_jobs(JobQuery {
                    dataset_id,
                    plan_id,
                    state: state.map(job_state),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&jobs, jobs.len(), page)?;
        }
        JobCommand::Status { id } => {
            let job = store
                .get_job(id)
                .await?
                .with_context(|| format!("job not found: {id}"))?;
            crate::presentation::print(&job)?;
        }
        JobCommand::Execution { id } => {
            let execution = store
                .get_generation_execution_spec(id)
                .await?
                .with_context(|| format!("generation execution specification not found: {id}"))?;
            crate::presentation::print(&execution)?;
        }
        JobCommand::Attempts { id } => {
            let attempts = store.list_generation_attempts(id).await?;
            crate::presentation::print(&attempts)?;
        }
        JobCommand::Prompt {
            id,
            cell_index,
            requested_count,
        } => {
            let job = store
                .get_job(id)
                .await?
                .with_context(|| format!("job not found: {id}"))?;
            let execution = store
                .get_generation_execution_spec(id)
                .await?
                .with_context(|| format!("generation execution specification not found: {id}"))?;
            let dataset = store
                .get_dataset(job.dataset_id)
                .await?
                .with_context(|| format!("dataset not found: {}", job.dataset_id))?;
            let plan = store
                .get_plan(job.plan_id)
                .await?
                .with_context(|| format!("plan not found: {}", job.plan_id))?;
            let planned = plan.cells.get(cell_index).with_context(|| {
                format!(
                    "cell index {cell_index} is outside plan range 0..{}",
                    plan.cells.len()
                )
            })?;
            let assignment = store
                .get_generation_semantics(id)
                .await?
                .with_context(|| format!("generation semantic assignment not found: {id}"))?;
            let expected_template = if execution.construction_plan.is_some() {
                PromptBuilder::template_identity()?
            } else {
                PromptBuilder::legacy_template_identity()?
            };
            anyhow::ensure!(
                assignment.context.fingerprint == execution.semantic_context_fingerprint
                    && assignment.context.reproduce_fingerprint()?
                        == assignment.context.fingerprint
                    && execution.prompt_template == expected_template,
                "job execution provenance failed its integrity check"
            );
            let default_count = execution
                .initial_needs
                .iter()
                .find(|need| need.planned.cell == planned.cell)
                .map_or(1, |need| need.remaining_count.max(1))
                .min(execution.policy.batch_size);
            let count = requested_count.unwrap_or(default_count);
            anyhow::ensure!(count > 0, "requested count must be greater than zero");
            let prompt_builder = PromptBuilder::with_semantics(assignment.context);
            let attempts = store.list_generation_attempts(id).await?;
            let start_index = attempts
                .iter()
                .filter(|attempt| attempt.cell == planned.cell)
                .map(|attempt| {
                    if attempt.kind
                        == generation_core::jobs::GenerationAttemptKind::DeterministicConstruction
                        && attempt.state != generation_core::jobs::GenerationAttemptState::Succeeded
                    {
                        0
                    } else {
                        u64::from(attempt.requested_count)
                    }
                })
                .sum();
            let (provider_required, prepared_rows, completed_rows, request) =
                if let Some(construction_plan) = &execution.construction_plan {
                    let construction = construction_plan.compile()?;
                    let prepared =
                        construction.prepare(planned.cell.clone(), start_index, count)?;
                    if prepared.requires_llm() {
                        let rows = prepared.rows.clone();
                        (
                            true,
                            rows,
                            None,
                            Some(prompt_builder.build_hybrid(
                                &dataset,
                                prepared,
                                execution.parameters.clone(),
                                &[],
                            )),
                        )
                    } else {
                        let rows = prepared.rows.clone();
                        let completed = construction.complete(&prepared, Vec::new())?;
                        (false, rows, Some(completed), None)
                    }
                } else {
                    (
                        true,
                        Vec::new(),
                        None,
                        Some(prompt_builder.build(
                            &dataset,
                            planned.cell.clone(),
                            count,
                            execution.parameters.clone(),
                            &[],
                        )),
                    )
                };
            crate::presentation::print(&serde_json::json!({
                "execution_fingerprint": execution.fingerprint,
                "prompt_template": execution.prompt_template,
                "construction_plan": execution.construction_plan,
                "start_index": start_index,
                "provider_required": provider_required,
                "prepared_rows": prepared_rows,
                "completed_rows": completed_rows,
                "request": request,
            }))?;
        }
        JobCommand::Cancel { id } => {
            let changed = store.request_job_cancellation(id).await?;
            crate::presentation::print(&serde_json::json!({"cancel_requested": changed}))?;
        }
    }
    Ok(())
}

const fn job_state(state: JobStateArg) -> JobState {
    match state {
        JobStateArg::Queued => JobState::Queued,
        JobStateArg::Running => JobState::Running,
        JobStateArg::Completed => JobState::Completed,
        JobStateArg::Failed => JobState::Failed,
        JobStateArg::Cancelled => JobState::Cancelled,
    }
}

pub async fn coverage(plan_id: Uuid, store: &SqliteStore) -> anyhow::Result<()> {
    let plan = store
        .get_plan(plan_id)
        .await?
        .with_context(|| format!("plan not found: {plan_id}"))?;
    let coverage = calculate_coverage(&plan, &store.dataset_cell_counts(plan.dataset_id).await?);
    crate::presentation::print(&coverage)?;
    Ok(())
}

pub async fn rows(args: RowsArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let status = args.status.map(|status| match status {
        RowStatus::Accepted => ValidationStatus::Accepted,
        RowStatus::Rejected => ValidationStatus::Rejected,
    });
    let rows = store
        .list_rows(RowQuery {
            dataset_id: args.dataset_id,
            job_id: args.job_id,
            status,
            limit: args.limit,
            offset: args.offset,
        })
        .await?;
    crate::presentation::print_page(
        &rows,
        rows.len(),
        crate::cli::PageArgs {
            limit: args.limit,
            offset: args.offset,
            summary: args.summary,
        },
    )?;
    Ok(())
}
