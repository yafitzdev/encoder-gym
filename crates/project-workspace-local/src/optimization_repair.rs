//! Passive repair-plan reads from verified immutable decisions and receipts.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, ensure};
use encoder_optimization_core::{
    agent::InspectionItem,
    repair_plan::{DatasetRepairPlan, recorded_plan},
};
use serde::Serialize;
use sqlx::Connection;
use uuid::Uuid;

use crate::{
    connect, dataset_versions, optimization_agent, optimization_dataset, optimization_generation,
    optimization_iterations,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairPublication {
    pub dataset_version_id: Uuid,
    pub added: u64,
    pub removed: u64,
    pub rows: u64,
    pub cross_batch_duplicates: u64,
}

/// Native inspection content is not a presentation contract. The composition
/// root must use the task adapter's bounded evidence projection.
pub struct RepairPlanRead {
    pub plan: DatasetRepairPlan,
    pub input_rows: u64,
    pub publication: Option<RepairPublication>,
    pub evidence: Vec<InspectionItem>,
}

pub async fn read(folder: &Path, run_id: Uuid) -> Result<BTreeMap<u32, RepairPlanRead>> {
    let mut db = connect(folder, true, false).await?;
    let mut snapshot = db.begin().await?;
    // Establish the receipt snapshot before loading the verified iteration
    // list. Concurrent advancement may add a newer, empty-at-this-snapshot
    // iteration, but cannot make captured generation look unbound.
    sqlx::query("SELECT name FROM sqlite_master LIMIT 1")
        .fetch_optional(&mut *snapshot)
        .await?;
    let iterations = optimization_iterations::list(folder, run_id).await?;
    if iterations.is_empty() {
        snapshot.rollback().await?;
        db.close().await?;
        return Ok(BTreeMap::new());
    }
    let generation = optimization_generation::read_calls(&mut snapshot, run_id).await?;
    let mut by_iteration = BTreeMap::<u32, Vec<_>>::new();
    for record in generation {
        by_iteration
            .entry(record.0.iteration)
            .or_default()
            .push(record);
    }
    let mut result = BTreeMap::new();
    for iteration in iterations {
        let number = iteration.scope.iteration;
        let calls = optimization_agent::read_history(&mut snapshot, run_id, Some(number)).await?;
        let records = by_iteration.remove(&number).unwrap_or_default();
        let publication =
            optimization_dataset::read_publication(&mut snapshot, run_id, number).await?;
        let Some(recorded) = recorded_plan(&iteration.scope, &calls, &records)? else {
            ensure!(
                publication.is_none(),
                "Published changes have no repair decision"
            );
            continue;
        };
        let parent = dataset_versions::inspect(folder, iteration.dataset.id).await?;
        ensure!(
            parent.reference() == iteration.dataset,
            "Repair plan input dataset changed"
        );
        let plan = recorded.plan;
        let published = if let Some(publication) = publication {
            let removed: Vec<_> = plan
                .proposal
                .removals
                .iter()
                .map(|row| row.row_id.clone())
                .collect();
            ensure!(
                !plan.proposal.stop
                    && publication.proposal_fingerprint == plan.proposal_fingerprint
                    && publication.parent == iteration.dataset
                    && publication.removed == removed
                    && plan.generation.iter().all(|target| target.unresolved == 0),
                "Published changes differ from the recorded repair plan"
            );
            let admitted: Vec<_> = records
                .iter()
                .filter_map(|(task, call, outcome)| {
                    outcome
                        .as_ref()
                        .and_then(|outcome| outcome.admission.as_ref())
                        .map(|admission| (task, call, admission))
                })
                .collect();
            let expected_rejections: u64 = admitted
                .iter()
                .map(|(_, _, value)| value.rejected.len() as u64)
                .sum();
            let expected_accepted: usize = admitted
                .iter()
                .map(|(_, _, value)| value.accepted.len())
                .sum();
            ensure!(
                publication.generation_rejections == expected_rejections
                    && publication.generated.len() == expected_accepted,
                "Publication admission counts changed"
            );
            let mut links = std::collections::BTreeSet::new();
            for link in &publication.generated {
                ensure!(
                    links.insert((link.call_id, link.row_index))
                        && admitted
                            .iter()
                            .any(|(task, call, admission)| task.id == link.task_id
                                && call.id == link.call_id
                                && admission
                                    .accepted
                                    .iter()
                                    .any(|row| row.index == link.row_index
                                        && row.fingerprint == link.content_fingerprint)),
                    "Published row has no exact generation admission"
                );
            }
            let version = dataset_versions::inspect(folder, publication.version.id).await?;
            let fork = dataset_versions::inspect(
                folder,
                version
                    .parent
                    .as_ref()
                    .context("Repair dataset branch is missing")?
                    .id,
            )
            .await?;
            ensure!(
                Some(fork.reference()) == version.parent
                    && fork.parent.as_ref() == Some(&iteration.dataset)
                    && fork.changes.is_empty()
                    && fork.members == parent.members,
                "Published repair dataset has another source branch"
            );
            let added = publication
                .generated
                .iter()
                .filter(|link| link.source_record.is_some())
                .count() as u64;
            ensure!(
                version.reference() == publication.version
                    && version.changes.removed == removed
                    && version.changes.replaced.is_empty()
                    && version.changes.added.len() as u64 == added
                    && publication.cross_batch_duplicates
                        == publication.generated.len() as u64 - added
                    && version.changes.apply(&parent.members)? == version.members,
                "Published dataset membership differs from its repair receipt"
            );
            for link in publication
                .generated
                .iter()
                .filter(|link| link.source_record.is_some())
            {
                ensure!(
                    version
                        .changes
                        .added
                        .iter()
                        .any(
                            |member| Some(member.source.import_id) == publication.import_id
                                && Some(member.source.record) == link.source_record
                                && member.content_fingerprint == link.content_fingerprint
                        ),
                    "Published dataset source differs from its admitted row"
                );
            }
            Some(RepairPublication {
                dataset_version_id: version.id,
                added,
                removed: removed.len() as u64,
                rows: version.members.len() as u64,
                cross_batch_duplicates: publication.cross_batch_duplicates,
            })
        } else {
            None
        };
        result.insert(
            number,
            RepairPlanRead {
                plan,
                input_rows: parent.members.len() as u64,
                publication: published,
                evidence: recorded.evidence,
            },
        );
    }
    ensure!(
        by_iteration.is_empty(),
        "Generation refers to an unbound iteration"
    );
    snapshot.rollback().await?;
    db.close().await?;
    Ok(result)
}
