use anyhow::{Context, bail};
use dataset_core::{domain::SnapshotSplit, ports::SnapshotStore};
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    governance::{
        CohortDisposition, CohortOrigin, CohortRole, CohortRoleDecision, DisclosureLevel,
        EvaluationCohort, EvidenceExposure, EvidenceExposureRequest, ExposurePurpose,
        summarize_exposure_risk,
    },
    ports::{CohortQuery, ExposureQuery, GovernanceStore},
};

use crate::cli::{
    CohortCommand, CohortOriginArg, CohortRoleArg, DisclosureLevelArg, ExposureCommand,
    ExposurePurposeArg, SnapshotSplitArg,
};

pub async fn cohort(command: CohortCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        CohortCommand::Create {
            snapshot_id,
            name,
            split,
            origin,
            role,
            reason,
        } => {
            let snapshot = store
                .get_snapshot(snapshot_id)
                .await?
                .with_context(|| format!("snapshot not found: {snapshot_id}"))?;
            let cohort = EvaluationCohort::new(
                name,
                snapshot.id,
                snapshot.fingerprint,
                snapshot_split(split),
                cohort_origin(origin),
            )?;
            let role = CohortRoleDecision::initial(&cohort, cohort_role(role), reason)?;
            store.create_cohort(&cohort, &role).await?;
            crate::presentation::print(&serde_json::json!({
                "cohort": cohort,
                "role": role,
            }))
        }
        CohortCommand::List { snapshot_id, page } => {
            let cohorts = store
                .query_cohorts(CohortQuery {
                    snapshot_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&cohorts, cohorts.len(), page)
        }
        CohortCommand::Show { id } => {
            let cohort = load_cohort(store, id).await?;
            let role = store
                .get_current_cohort_role(id)
                .await?
                .context("cohort has no role decision")?;
            crate::presentation::print(&serde_json::json!({
                "cohort": cohort,
                "current_role": role,
            }))
        }
        CohortCommand::Assign { id, role, reason } => {
            let current = store
                .get_current_cohort_role(id)
                .await?
                .with_context(|| format!("cohort not found or has no role: {id}"))?;
            let decision = CohortRoleDecision::transition(
                &current,
                cohort_role(role),
                CohortDisposition::Active,
                reason,
            )?;
            store.append_cohort_role(&decision).await?;
            crate::presentation::print(&decision)
        }
        CohortCommand::Retire { id, reason } => {
            let current = store
                .get_current_cohort_role(id)
                .await?
                .with_context(|| format!("cohort not found or has no role: {id}"))?;
            let decision = CohortRoleDecision::transition(
                &current,
                current.role,
                CohortDisposition::Retired,
                reason,
            )?;
            store.append_cohort_role(&decision).await?;
            crate::presentation::print(&decision)
        }
        CohortCommand::History { id } => {
            load_cohort(store, id).await?;
            crate::presentation::print(&store.list_cohort_role_history(id).await?)
        }
    }
}

pub async fn exposure(command: ExposureCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        ExposureCommand::Record {
            cohort_id,
            evaluation_run_id,
            purpose,
            disclosure,
            adaptation_eligible,
            note,
            retirement_reason,
        } => {
            let cohort = load_cohort(store, cohort_id).await?;
            let role = store
                .get_current_cohort_role(cohort_id)
                .await?
                .context("cohort has no role decision")?;
            let exposure = EvidenceExposure::new(
                &cohort,
                &role,
                EvidenceExposureRequest {
                    evaluation_run_id,
                    workflow_run_id: None,
                    workflow_iteration: None,
                    purpose: exposure_purpose(purpose),
                    disclosure: disclosure_level(disclosure),
                    adaptation_eligible,
                    note,
                },
            )?;
            let resolution = if exposure.requires_retirement {
                let reason = retirement_reason.context(
                    "--retirement-reason is required because this sealed disclosure retires the cohort",
                )?;
                Some(CohortRoleDecision::transition(
                    &role,
                    role.role,
                    CohortDisposition::Retired,
                    reason,
                )?)
            } else {
                if retirement_reason.is_some() {
                    bail!("--retirement-reason is valid only when sealed disclosure requires it");
                }
                None
            };
            store
                .append_exposure(&exposure, resolution.as_ref())
                .await?;
            crate::presentation::print(&serde_json::json!({
                "exposure": exposure,
                "role_resolution": resolution,
            }))
        }
        ExposureCommand::List {
            cohort_id,
            purpose,
            page,
        } => {
            load_cohort(store, cohort_id).await?;
            let exposures = store
                .query_exposures(ExposureQuery {
                    cohort_id,
                    purpose: purpose.map(exposure_purpose),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&exposures, exposures.len(), page)
        }
        ExposureCommand::Risk { cohort_id } => {
            load_cohort(store, cohort_id).await?;
            let exposures = store
                .query_exposures(ExposureQuery {
                    cohort_id,
                    purpose: None,
                    limit: 10_000,
                    offset: 0,
                })
                .await?;
            crate::presentation::print(&summarize_exposure_risk(cohort_id, &exposures)?)
        }
    }
}

async fn load_cohort(store: &SqliteStore, id: uuid::Uuid) -> anyhow::Result<EvaluationCohort> {
    store
        .get_cohort(id)
        .await?
        .with_context(|| format!("cohort not found: {id}"))
}

const fn snapshot_split(value: SnapshotSplitArg) -> SnapshotSplit {
    match value {
        SnapshotSplitArg::Train => SnapshotSplit::Train,
        SnapshotSplitArg::Validation => SnapshotSplit::Validation,
        SnapshotSplitArg::Test => SnapshotSplit::Test,
    }
}

const fn cohort_origin(value: CohortOriginArg) -> CohortOrigin {
    match value {
        CohortOriginArg::InternalSnapshot => CohortOrigin::InternalSnapshot,
        CohortOriginArg::ExternalBenchmark => CohortOrigin::ExternalBenchmark,
    }
}

const fn cohort_role(value: CohortRoleArg) -> CohortRole {
    match value {
        CohortRoleArg::Training => CohortRole::Training,
        CohortRoleArg::Development => CohortRole::Development,
        CohortRoleArg::Diagnostic => CohortRole::Diagnostic,
        CohortRoleArg::SealedAcceptance => CohortRole::SealedAcceptance,
        CohortRoleArg::ExternalBenchmark => CohortRole::ExternalBenchmark,
    }
}

const fn exposure_purpose(value: ExposurePurposeArg) -> ExposurePurpose {
    match value {
        ExposurePurposeArg::Training => ExposurePurpose::Training,
        ExposurePurposeArg::DevelopmentEvaluation => ExposurePurpose::DevelopmentEvaluation,
        ExposurePurposeArg::Diagnosis => ExposurePurpose::Diagnosis,
        ExposurePurposeArg::Comparison => ExposurePurpose::Comparison,
        ExposurePurposeArg::Acceptance => ExposurePurpose::Acceptance,
        ExposurePurposeArg::ManualInspection => ExposurePurpose::ManualInspection,
        ExposurePurposeArg::Advisor => ExposurePurpose::Advisor,
        ExposurePurposeArg::Optimization => ExposurePurpose::Optimization,
    }
}

const fn disclosure_level(value: DisclosureLevelArg) -> DisclosureLevel {
    match value {
        DisclosureLevelArg::Aggregate => DisclosureLevel::Aggregate,
        DisclosureLevelArg::Slices => DisclosureLevel::Slices,
        DisclosureLevelArg::Predictions => DisclosureLevel::Predictions,
        DisclosureLevelArg::RowContent => DisclosureLevel::RowContent,
    }
}
