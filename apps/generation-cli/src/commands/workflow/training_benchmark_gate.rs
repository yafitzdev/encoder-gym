use std::collections::BTreeSet;

use anyhow::{Context, ensure};
use dataset_core::{domain::SnapshotSplit, ports::SnapshotStore, splitting::verify_snapshot};
use workflow_core::{
    contamination::{
        CohortContaminationInput, ContaminationError, ContaminationMember, ContaminationPolicy,
        check_contamination,
    },
    governance::{
        CohortDisposition, CohortOrigin, CohortRole, CohortRoleDecision, EvaluationCohort,
    },
    ports::{
        CohortQuery, GovernanceStore, TrainingBenchmarkCheckQuery, TrainingBenchmarkCheckStore,
    },
    training_benchmark::{
        TRAINING_BENCHMARK_CHECK_PROTOCOL, TrainingBenchmarkCheck, TrainingCohortEvidence,
        TrainingInputProtocol, build_training_benchmark_check,
    },
    workflow::WorkflowDefinition,
};

use super::{SqliteStore, WorkflowBenchmarkAuthority, load_bundle_contamination_inputs};

const COHORT_PAGE_SIZE: u32 = 256;

pub(super) enum GateResult {
    Checked(Box<TrainingBenchmarkCheck>),
    DeterministicallyInvalid(String),
}

pub(super) async fn ensure_check(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    snapshot_id: uuid::Uuid,
    authority: &WorkflowBenchmarkAuthority,
) -> anyhow::Result<GateResult> {
    let snapshot = store
        .get_snapshot(snapshot_id)
        .await?
        .context("workflow training snapshot not found")?;
    let members = store.list_snapshot_members(snapshot_id).await?;
    verify_snapshot(&snapshot, &members).context("workflow training snapshot is invalid")?;
    ensure!(
        snapshot.source_dataset_id == definition.dataset_id,
        "workflow training snapshot belongs to another dataset"
    );

    let existing = store
        .query_training_benchmark_checks(TrainingBenchmarkCheckQuery {
            training_snapshot_id: Some(snapshot.id),
            benchmark_bundle_id: Some(authority.bundle.id),
            status: None,
            protocol: Some(TrainingInputProtocol::TrainAndValidationV1),
            check_protocol_version: Some(TRAINING_BENCHMARK_CHECK_PROTOCOL.into()),
            limit: 2,
            offset: 0,
        })
        .await?;
    ensure!(
        existing.len() <= 1,
        "multiple training-benchmark checks exist for one immutable authority"
    );
    if let Some(existing) = existing.first() {
        let check = store
            .get_executable_training_benchmark_check(existing.id)
            .await?
            .context("training-benchmark check disappeared")?;
        ensure!(
            check.training_snapshot_fingerprint == snapshot.fingerprint
                && check.benchmark_bundle_fingerprint == authority.bundle.fingerprint,
            "training-benchmark check does not match current workflow authority"
        );
        return Ok(GateResult::Checked(Box::new(check)));
    }

    let desired_splits = [SnapshotSplit::Train, SnapshotSplit::Validation]
        .into_iter()
        .filter(|split| members.iter().any(|member| member.split == *split))
        .collect::<Vec<_>>();
    ensure!(
        desired_splits.first() == Some(&SnapshotSplit::Train),
        "workflow training snapshot has no train members"
    );
    let persisted = all_snapshot_cohorts(store, snapshot.id).await?;
    let mut evidence = Vec::with_capacity(desired_splits.len());
    let mut new_evidence = Vec::new();
    for split in desired_splits {
        let mut reusable = Vec::new();
        for cohort in persisted.iter().filter(|cohort| {
            cohort.snapshot_fingerprint == snapshot.fingerprint
                && cohort.split == split
                && cohort.origin == CohortOrigin::InternalSnapshot
        }) {
            let history = store.list_cohort_role_history(cohort.id).await?;
            if !history.iter().any(|role| role.role == CohortRole::Training) {
                continue;
            }
            let current = history
                .last()
                .context("training cohort has no role history")?;
            ensure!(
                current.role == CohortRole::Training
                    && current.disposition == CohortDisposition::Active,
                "existing training cohort {} for {split:?} is no longer active",
                cohort.id
            );
            reusable.push(TrainingCohortEvidence {
                cohort: cohort.clone(),
                role: current.clone(),
            });
        }
        ensure!(
            reusable.len() <= 1,
            "multiple active Training cohorts bind snapshot {} split {split:?}",
            snapshot.id
        );
        let selected = if let Some(value) = reusable.pop() {
            value
        } else {
            let cohort = EvaluationCohort::new(
                format!("workflow training {} {split:?}", snapshot.id),
                snapshot.id,
                snapshot.fingerprint.clone(),
                split,
                CohortOrigin::InternalSnapshot,
            )?;
            let role = CohortRoleDecision::initial(
                &cohort,
                CohortRole::Training,
                "trainer-visible workflow snapshot population",
            )?;
            let value = TrainingCohortEvidence { cohort, role };
            new_evidence.push(value.clone());
            value
        };
        evidence.push(selected);
    }

    let group_dimension = authority.global_report.group_dimension.as_deref();
    if let Some(dimension) = group_dimension {
        let missing = members
            .iter()
            .filter(|member| {
                matches!(
                    member.split,
                    SnapshotSplit::Train | SnapshotSplit::Validation
                ) && member
                    .dimensions
                    .get(dimension)
                    .is_none_or(|value| value.trim().is_empty())
            })
            .count();
        if missing > 0 {
            return Ok(GateResult::DeterministicallyInvalid(format!(
                "training-benchmark input invalid: {missing} trainer-visible member(s) have no value for benchmark group dimension {dimension}"
            )));
        }
    }
    let mut inputs = evidence
        .iter()
        .map(|value| CohortContaminationInput {
            cohort: value.cohort.clone(),
            role: value.role.clone(),
            members: members
                .iter()
                .filter(|member| member.split == value.cohort.split)
                .map(|member| ContaminationMember::from_snapshot_member(member, group_dimension))
                .collect(),
        })
        .collect::<Vec<_>>();
    let suites = std::iter::once(&authority.development)
        .chain(authority.sealed.iter())
        .collect::<Vec<_>>();
    inputs.extend(load_bundle_contamination_inputs(store, &suites, group_dimension).await?);
    let report = match check_contamination(
        inputs,
        authority.global_report.group_dimension.clone(),
        ContaminationPolicy::default(),
    ) {
        Ok(report) => report,
        Err(ContaminationError::MissingGroup { dimension, .. }) => {
            return Ok(GateResult::DeterministicallyInvalid(format!(
                "training-benchmark input invalid: a contamination participant has no value for benchmark group dimension {dimension}"
            )));
        }
        Err(error) => return Err(error.into()),
    };
    let check = build_training_benchmark_check(
        &snapshot,
        &members,
        TrainingInputProtocol::TrainAndValidationV1,
        evidence,
        &authority.bundle,
        &authority.global_report,
        &report,
    )?;
    store
        .create_training_benchmark_check(&new_evidence, &report, &check)
        .await?;
    let persisted = store
        .get_executable_training_benchmark_check(check.id)
        .await?
        .context("new training-benchmark check did not persist")?;
    ensure!(
        persisted.fingerprint == check.fingerprint,
        "persisted training-benchmark check differs from constructed evidence"
    );
    Ok(GateResult::Checked(Box::new(persisted)))
}

pub(super) async fn require_linked_clean_check(
    store: &SqliteStore,
    check_id: uuid::Uuid,
    check_fingerprint: &str,
    snapshot_id: uuid::Uuid,
    authority: &WorkflowBenchmarkAuthority,
) -> anyhow::Result<TrainingBenchmarkCheck> {
    let check = store
        .get_executable_training_benchmark_check(check_id)
        .await?
        .context("linked training-benchmark check not found")?;
    ensure!(
        check.fingerprint == check_fingerprint
            && check.training_snapshot_id == snapshot_id
            && check.benchmark_bundle_id == authority.bundle.id
            && check.benchmark_bundle_fingerprint == authority.bundle.fingerprint
            && check.training_allowed(),
        "linked training-benchmark check is not clean authority for this snapshot and bundle"
    );
    Ok(check)
}

async fn all_snapshot_cohorts(
    store: &SqliteStore,
    snapshot_id: uuid::Uuid,
) -> anyhow::Result<Vec<EvaluationCohort>> {
    let mut values = Vec::new();
    let mut offset = 0;
    loop {
        let page = store
            .query_cohorts(CohortQuery {
                snapshot_id: Some(snapshot_id),
                limit: COHORT_PAGE_SIZE,
                offset,
            })
            .await?;
        let returned = u32::try_from(page.len()).context("cohort page is too large")?;
        values.extend(page);
        if returned < COHORT_PAGE_SIZE {
            break;
        }
        offset = offset
            .checked_add(returned)
            .context("cohort offset overflow")?;
    }
    let ids = values
        .iter()
        .map(|cohort| cohort.id)
        .collect::<BTreeSet<_>>();
    ensure!(
        ids.len() == values.len(),
        "cohort query returned duplicates"
    );
    Ok(values)
}
