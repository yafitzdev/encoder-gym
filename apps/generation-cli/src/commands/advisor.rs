use anyhow::Context;
use synthetic_data_sqlite::SqliteStore;
use workflow_core::ports::{AdvisorStore, AdvisoryAssessmentQuery};

use crate::cli::AdvisorCommand;

pub async fn execute(command: AdvisorCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        AdvisorCommand::Show { id } => crate::presentation::print(
            &store
                .get_advisory_assessment(id)
                .await?
                .with_context(|| format!("advisory assessment not found: {id}"))?,
        ),
        AdvisorCommand::List {
            workflow_run_id,
            analysis_report_id,
            page,
        } => {
            let values = store
                .query_advisory_assessments(AdvisoryAssessmentQuery {
                    workflow_run_id,
                    analysis_report_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&values, values.len(), page)
        }
    }
}
