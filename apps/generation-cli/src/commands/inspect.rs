use anyhow::Context;
use generation_core::{
    coverage::calculate_coverage,
    domain::ValidationStatus,
    jobs::JobState,
    ports::{JobQuery, JobStore, PlanStore, RowQuery, RowStore},
};
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
