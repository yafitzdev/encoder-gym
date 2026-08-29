use std::path::Path;

use anyhow::Context;
use dataset_core::ports::SnapshotStore;
use serde::de::DeserializeOwned;
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    contamination::{
        CohortContaminationInput, ContaminationMember, ContaminationOverride, ContaminationStatus,
        check_contamination,
    },
    ports::{ContaminationQuery, ContaminationStore, GovernanceStore},
};

use crate::cli::{ContaminationCommand, ContaminationStatusArg};

pub async fn execute(command: ContaminationCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        ContaminationCommand::Check {
            cohort_ids,
            group_dimension,
            policy,
        } => {
            let mut inputs = Vec::with_capacity(cohort_ids.len());
            for cohort_id in cohort_ids {
                let cohort = store
                    .get_cohort(cohort_id)
                    .await?
                    .with_context(|| format!("cohort not found: {cohort_id}"))?;
                let role = store
                    .get_current_cohort_role(cohort_id)
                    .await?
                    .context("cohort has no current role")?;
                let members = store
                    .list_snapshot_members(cohort.snapshot_id)
                    .await?
                    .into_iter()
                    .filter(|member| member.split == cohort.split)
                    .map(|member| {
                        ContaminationMember::from_snapshot_member(
                            &member,
                            group_dimension.as_deref(),
                        )
                    })
                    .collect();
                inputs.push(CohortContaminationInput {
                    cohort,
                    role,
                    members,
                });
            }
            let policy = policy
                .as_deref()
                .map(read_document)
                .transpose()?
                .unwrap_or_default();
            let report = check_contamination(inputs, group_dimension, policy)?;
            store.create_contamination_report(&report).await?;
            crate::presentation::print(&report)
        }
        ContaminationCommand::Show { id } => {
            let report = require_report(store, id).await?;
            let override_record = store.get_contamination_override(id).await?;
            crate::presentation::print(&serde_json::json!({
                "report": report,
                "override": override_record,
                "eligible": report.status == ContaminationStatus::Clean || override_record.is_some(),
            }))
        }
        ContaminationCommand::List {
            cohort_id,
            status,
            page,
        } => {
            let reports = store
                .query_contamination_reports(ContaminationQuery {
                    cohort_id,
                    status: status.map(contamination_status),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&reports, reports.len(), page)
        }
        ContaminationCommand::Override {
            id,
            reason,
            approved_by,
        } => {
            let report = require_report(store, id).await?;
            let value = ContaminationOverride::new(&report, reason, approved_by)?;
            store.append_contamination_override(&value).await?;
            crate::presentation::print(&value)
        }
    }
}

async fn require_report(
    store: &SqliteStore,
    id: uuid::Uuid,
) -> anyhow::Result<workflow_core::contamination::ContaminationReport> {
    store
        .get_contamination_report(id)
        .await?
        .with_context(|| format!("contamination report not found: {id}"))
}

fn read_document<T: DeserializeOwned>(path: &Path) -> anyhow::Result<T> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    match path.extension().and_then(|value| value.to_str()) {
        Some("json") => serde_json::from_str(&contents)
            .with_context(|| format!("invalid JSON in {}", path.display())),
        _ => {
            toml::from_str(&contents).with_context(|| format!("invalid TOML in {}", path.display()))
        }
    }
}

const fn contamination_status(value: ContaminationStatusArg) -> ContaminationStatus {
    match value {
        ContaminationStatusArg::Clean => ContaminationStatus::Clean,
        ContaminationStatusArg::Blocked => ContaminationStatus::Blocked,
    }
}
