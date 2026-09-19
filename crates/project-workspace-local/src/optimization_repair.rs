//! Passive repair-plan reads from verified immutable decisions and receipts.
use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, Result, ensure};
use encoder_optimization_core::{
    agent::{AgentCallReservation, AgentTurnRecord, InspectionItem},
    generation::{
        AdmittedGenerationRow, GenerationCanaryObservation, GenerationCanaryPolicy,
        GenerationCanaryStatus, GenerationOutcome, GenerationPhase, GenerationReservation,
        GenerationTask, RepairNotExecuted, V3CanaryGate, V3CanaryStatus, canary_passed,
        evaluate_v3_canaries,
    },
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
    pub semantic_rejections: u64,
    pub coupled_rejections: u64,
    pub target_counts: Vec<optimization_dataset::RepairPublicationCounts>,
}

/// Native inspection content is not a presentation contract. The composition
/// root must use the task adapter's bounded evidence projection.
pub struct RepairPlanRead {
    pub plan: DatasetRepairPlan,
    pub input_rows: u64,
    pub publication: Option<RepairPublication>,
    pub evidence: Vec<InspectionItem>,
    pub canary: GenerationCanaryObservation,
    pub canary_rows: Vec<AdmittedGenerationRow>,
    pub v3_canary: Option<V3CanaryGate>,
    pub not_executed: Option<RepairNotExecuted>,
}

struct ReceiptSnapshot {
    iteration: project_workspace_core::optimization_iteration::ProjectOptimizationIteration,
    calls: Vec<(AgentCallReservation, Option<AgentTurnRecord>)>,
    generation: Vec<(
        GenerationTask,
        GenerationReservation,
        Option<GenerationOutcome>,
    )>,
    publication: Option<optimization_dataset::OptimizationDatasetPublication>,
    native_admissions: Vec<dataset_quality_core::native_assessment::NativeAdmissionRecord>,
    not_executed: Option<RepairNotExecuted>,
}

async fn receipt_snapshot(folder: &Path, run_id: Uuid) -> Result<BTreeMap<u32, ReceiptSnapshot>> {
    let mut db = connect(folder, true, false).await?;
    let mut snapshot = db.begin().await?;
    // Capture bindings and all receipts on ONE connection. Never open another
    // reader while holding this shared lock: a pending rollback-journal writer
    // would wait on us while our nested reader waits on that writer.
    let iterations = optimization_iterations::read(&mut snapshot, run_id).await?;
    let mut generation = BTreeMap::<u32, Vec<_>>::new();
    if !iterations.is_empty() {
        for record in optimization_generation::read_calls(&mut snapshot, run_id).await? {
            generation
                .entry(record.0.iteration)
                .or_default()
                .push(record);
        }
    }
    let mut receipts = BTreeMap::new();
    for iteration in iterations {
        let number = iteration.scope.iteration;
        receipts.insert(
            number,
            ReceiptSnapshot {
                iteration,
                calls: optimization_agent::read_history(&mut snapshot, run_id, Some(number))
                    .await?,
                generation: generation.remove(&number).unwrap_or_default(),
                publication: optimization_dataset::read_publication(&mut snapshot, run_id, number)
                    .await?,
                native_admissions: crate::optimization_native_review::read_admissions(
                    &mut snapshot,
                    run_id,
                    number,
                )
                .await?,
                not_executed: crate::optimization_repair_execution::read_not_executed(
                    &mut snapshot,
                    run_id,
                    number,
                )
                .await?,
            },
        );
    }
    ensure!(
        generation.is_empty(),
        "Generation refers to an unbound iteration"
    );
    snapshot.rollback().await?;
    db.close().await?;
    Ok(receipts)
}

pub async fn read(folder: &Path, run_id: Uuid) -> Result<BTreeMap<u32, RepairPlanRead>> {
    let mut receipts = receipt_snapshot(folder, run_id).await?;
    // Validate custody only AFTER releasing the receipt snapshot. A concurrent
    // append may add iterations, but cannot change any captured binding.
    let iterations = optimization_iterations::list(folder, run_id).await?;
    if receipts.is_empty() {
        return Ok(BTreeMap::new());
    }
    let run = crate::optimization_runs::show(folder, run_id).await?;
    let launches = crate::optimization_launch::list(folder).await?;
    let launch = launches
        .iter()
        .find(|launch| launch.id.to_string() == run.run.launch.id)
        .context("Repair plan launch is missing")?;
    run.run.validate(launch)?;
    let canary_policy = launch
        .scope
        .agentic
        .as_ref()
        .and_then(|settings| settings.generation_canary);
    let mut result = BTreeMap::new();
    for iteration in iterations {
        let number = iteration.scope.iteration;
        let Some(receipt) = receipts.remove(&number) else {
            continue;
        };
        ensure!(
            receipt.iteration == iteration,
            "Repair plan iteration changed"
        );
        let calls = receipt.calls;
        let records = receipt.generation;
        let publication = receipt.publication;
        let native_admissions = receipt.native_admissions;
        let not_executed = receipt.not_executed;
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
        let mut canary = GenerationCanaryObservation {
            policy: canary_policy,
            status: if canary_policy.is_none() {
                GenerationCanaryStatus::Disabled
            } else if plan.proposal.additions.is_empty() {
                GenerationCanaryStatus::NotRequired
            } else {
                GenerationCanaryStatus::Pending
            },
            call_id: None,
            requested: if canary_policy.is_some() {
                plan.proposal
                    .additions
                    .first()
                    .map_or(0, |target| target.count.min(8))
            } else {
                0
            },
            admitted: 0,
            rejected: Vec::new(),
        };
        let mut canary_rows = Vec::new();
        let mut v3_canary = None;
        if canary_policy == Some(GenerationCanaryPolicy::PerCombinationSemanticV3) {
            let mut completed = BTreeMap::new();
            for (task, _, outcome) in records.iter().filter(|(task, _, _)| {
                task.execution_v3
                    .as_ref()
                    .is_some_and(|value| value.phase == GenerationPhase::Canary)
            }) {
                if let Some(outcome) = outcome.as_ref().filter(|value| !value.interrupted) {
                    ensure!(
                        completed
                            .insert(task.id, (task.clone(), outcome.clone()))
                            .is_none(),
                        "Protocol V3 canary completed more than once"
                    );
                }
            }
            if !completed.is_empty() {
                let (tasks, outcomes): (Vec<_>, Vec<_>) = completed.into_values().unzip();
                let decisions = native_admissions
                    .iter()
                    .filter(|record| {
                        tasks.iter().any(|task| {
                            record
                                .authority
                                .row_id
                                .starts_with(&format!("generation:{}:", task.id))
                        })
                    })
                    .map(|record| (record.authority.row_id.clone(), record.admission.admitted()))
                    .collect();
                let gate = evaluate_v3_canaries(&tasks, &outcomes, &decisions)?;
                canary.requested = gate.units.iter().map(|unit| unit.requested).sum();
                canary.admitted = gate
                    .units
                    .iter()
                    .map(|unit| unit.semantically_admitted)
                    .sum();
                canary.status = if gate.status == V3CanaryStatus::Passed {
                    GenerationCanaryStatus::Passed
                } else {
                    GenerationCanaryStatus::Rejected
                };
                canary_rows = outcomes
                    .iter()
                    .flat_map(|outcome| {
                        outcome
                            .admission
                            .iter()
                            .flat_map(|admission| admission.accepted.iter().cloned())
                    })
                    .collect();
                v3_canary = Some(gate);
            }
            if let Some(stopped) = &not_executed {
                ensure!(
                    v3_canary.as_ref() == Some(&stopped.canary)
                        && publication.is_none()
                        && records.iter().all(|(task, _, _)| {
                            task.execution_v3
                                .as_ref()
                                .is_some_and(|value| value.phase == GenerationPhase::Canary)
                        }),
                    "Canary-rejected repair continued or its receipt changed"
                );
            } else {
                ensure!(
                    publication.is_none()
                        || v3_canary
                            .as_ref()
                            .is_some_and(|gate| gate.status == V3CanaryStatus::Passed),
                    "Dataset was published without a passed protocol V3 canary"
                );
            }
        } else if canary_policy.is_some() {
            if let Some((task, call, outcome)) = records
                .iter()
                .filter(|(task, _, _)| task.target_index == 0 && task.first_row == 0)
                .max_by_key(|(_, call, _)| call.attempt)
            {
                canary.call_id = Some(call.id);
                if let Some(outcome) = outcome {
                    canary.status = if outcome.interrupted {
                        GenerationCanaryStatus::Interrupted
                    } else if canary_passed(task, outcome)? {
                        GenerationCanaryStatus::Passed
                    } else {
                        GenerationCanaryStatus::Rejected
                    };
                    if let Some(admission) = &outcome.admission {
                        canary.admitted = admission.accepted.len() as u32;
                        canary.rejected.clone_from(&admission.rejected);
                        canary_rows.clone_from(&admission.accepted);
                    }
                }
            }
            ensure!(
                canary.status == GenerationCanaryStatus::Passed
                    || records
                        .iter()
                        .all(|(task, _, _)| task.target_index == 0 && task.first_row == 0),
                "Generation continued without a passed canary"
            );
            ensure!(
                publication.is_none()
                    || matches!(
                        canary.status,
                        GenerationCanaryStatus::Passed | GenerationCanaryStatus::NotRequired
                    ),
                "Dataset was published without a passed canary"
            );
            ensure!(
                not_executed.is_none(),
                "Historical canary policy has a protocol V3 stop receipt"
            );
        } else {
            ensure!(
                not_executed.is_none(),
                "Disabled canary policy has a stop receipt"
            );
        }
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
            ensure!(
                publication.target_counts.is_empty()
                    || (publication
                        .target_counts
                        .iter()
                        .map(|counts| counts.requested)
                        .sum::<u64>()
                        == plan
                            .proposal
                            .additions
                            .iter()
                            .map(|target| u64::from(target.count))
                            .sum::<u64>()
                        && publication
                            .target_counts
                            .iter()
                            .map(|counts| counts.structurally_admitted)
                            .sum::<u64>()
                            == expected_accepted as u64
                        && publication
                            .target_counts
                            .iter()
                            .map(|counts| counts.published)
                            .sum::<u64>()
                            == publication
                                .generated
                                .iter()
                                .filter(|row| row.source_record.is_some())
                                .count() as u64),
                "Publication per-target counts changed"
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
                        == publication.generated.len() as u64
                            - added
                            - publication.semantic_rejections
                            - publication.coupled_rejections
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
                semantic_rejections: publication.semantic_rejections,
                coupled_rejections: publication.coupled_rejections,
                target_counts: publication.target_counts,
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
                canary,
                canary_rows,
                v3_canary,
                not_executed,
            },
        );
    }
    ensure!(
        receipts.is_empty(),
        "Repair receipts refer to an unbound iteration"
    );
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn receipt_snapshot_releases_read_lock_before_custody_reads() {
        let folder = tempfile::tempdir().unwrap();
        let mut writer = connect(folder.path(), false, true).await.unwrap();
        sqlx::query("CREATE TABLE project_optimization_iterations(id TEXT, run_id TEXT, iteration INTEGER, scope_fingerprint TEXT, fingerprint TEXT, metadata_json TEXT, created_at TEXT)")
            .execute(&mut writer).await.unwrap();
        let before = std::fs::read(folder.path().join(crate::DATABASE)).unwrap();
        assert!(
            receipt_snapshot(folder.path(), Uuid::new_v4())
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            before,
            std::fs::read(folder.path().join(crate::DATABASE)).unwrap()
        );
        sqlx::query("PRAGMA busy_timeout=0")
            .execute(&mut writer)
            .await
            .unwrap();
        // A leaked shared snapshot would reject this immediately. Subsequent
        // custody/model/dataset reads must not keep a pending writer waiting.
        let transaction = writer.begin_with("BEGIN EXCLUSIVE").await.unwrap();
        transaction.rollback().await.unwrap();
        writer.close().await.unwrap();
    }
}
