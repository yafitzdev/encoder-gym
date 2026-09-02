use chrono::{DateTime, Utc};
use encoder_experiment_core::ports::ExperimentStore;
use encoder_repair_core::{
    ports::{
        BoxFuture, NativeRepairQualityStore, NativeRepairTrainingStore, RepairEvidenceStore,
        RepairEvidenceStoreError,
    },
    training::NativeRepairTrainingSnapshot,
};
use uuid::Uuid;

use crate::SqliteExperimentStore;

#[derive(sqlx::FromRow)]
struct TrainingSnapshotStorageRow {
    specification_fingerprint: String,
    fingerprint: String,
    proposal_id: Uuid,
    selection_id: Uuid,
    execution_project_id: Uuid,
    combined_membership_fingerprint: String,
    artifact_json: String,
    created_at: DateTime<Utc>,
}

impl NativeRepairTrainingStore for SqliteExperimentStore {
    fn create_native_repair_training_snapshot(
        &self,
        snapshot: NativeRepairTrainingSnapshot,
    ) -> BoxFuture<'_, Result<NativeRepairTrainingSnapshot, RepairEvidenceStoreError>> {
        Box::pin(async move {
            self.validate_training_snapshot(&snapshot).await?;
            if let Some(existing) = self
                .get_native_repair_training_snapshot_for_selection(snapshot.selection.id)
                .await?
            {
                if existing.specification_fingerprint != snapshot.specification_fingerprint {
                    return Err(RepairEvidenceStoreError(
                        "native delta selection already has a different training snapshot".into(),
                    ));
                }
                return Ok(existing);
            }
            let artifact_json = serde_json::to_string(&snapshot).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_native_repair_training_snapshots \
                 (id, specification_fingerprint, fingerprint, proposal_id, selection_id, \
                  execution_project_snapshot_id, combined_membership_fingerprint, artifact_json, \
                  created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(snapshot.id)
            .bind(&snapshot.specification_fingerprint)
            .bind(&snapshot.fingerprint)
            .bind(snapshot.proposal.id)
            .bind(snapshot.selection.id)
            .bind(snapshot.execution_project.id)
            .bind(&snapshot.combined_membership_fingerprint)
            .bind(artifact_json)
            .bind(snapshot.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(snapshot)
        })
    }

    fn get_native_repair_training_snapshot(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<NativeRepairTrainingSnapshot>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let stored: Option<TrainingSnapshotStorageRow> = sqlx::query_as(
                "SELECT specification_fingerprint, fingerprint, proposal_id, selection_id, \
                 execution_project_snapshot_id AS execution_project_id, \
                 combined_membership_fingerprint, artifact_json, created_at \
                 FROM encoder_native_repair_training_snapshots WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(stored) = stored else {
                return Ok(None);
            };
            let value: NativeRepairTrainingSnapshot =
                serde_json::from_str(&stored.artifact_json).map_err(store_error)?;
            if value.id != id
                || value.specification_fingerprint != stored.specification_fingerprint
                || value.fingerprint != stored.fingerprint
                || value.proposal.id != stored.proposal_id
                || value.selection.id != stored.selection_id
                || value.execution_project.id != stored.execution_project_id
                || value.combined_membership_fingerprint != stored.combined_membership_fingerprint
                || value.created_at != stored.created_at
            {
                return Err(RepairEvidenceStoreError(
                    "native repair training snapshot storage envelope changed".into(),
                ));
            }
            self.validate_training_snapshot(&value).await?;
            Ok(Some(value))
        })
    }

    fn get_native_repair_training_snapshot_for_selection(
        &self,
        selection_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<NativeRepairTrainingSnapshot>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_native_repair_training_snapshots WHERE selection_id = ?",
            )
            .bind(selection_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match id {
                Some(id) => self.get_native_repair_training_snapshot(id).await,
                None => Ok(None),
            }
        })
    }
}

impl SqliteExperimentStore {
    async fn validate_training_snapshot(
        &self,
        snapshot: &NativeRepairTrainingSnapshot,
    ) -> Result<(), RepairEvidenceStoreError> {
        let proposal = self
            .get_proposal(snapshot.proposal.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("repair proposal is missing".into()))?;
        let candidate_set = self
            .get_native_delta_candidate_set(snapshot.candidate_set.id)
            .await?
            .ok_or_else(|| {
                RepairEvidenceStoreError("native delta candidate set is missing".into())
            })?;
        let report = self
            .get_native_delta_report(snapshot.report.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("native delta report is missing".into()))?;
        let reviews = self.list_native_delta_reviews(report.id).await?;
        let approval_index = reviews
            .iter()
            .position(|value| value.id == snapshot.approval.id)
            .ok_or_else(|| RepairEvidenceStoreError("native delta approval is missing".into()))?;
        if approval_index + 1 != reviews.len() {
            return Err(RepairEvidenceStoreError(
                "training snapshot does not bind the frozen latest delta approval".into(),
            ));
        }
        let selection = self
            .get_native_delta_selection(snapshot.selection.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("native delta selection is missing".into()))?;
        let project = self
            .get_project(snapshot.execution_project.id)
            .await
            .map_err(store_error)?
            .ok_or_else(|| {
                RepairEvidenceStoreError("repair execution project is missing".into())
            })?;
        snapshot
            .validate_against(
                &project,
                &proposal,
                &candidate_set,
                &report,
                &reviews[approval_index],
                approval_index
                    .checked_sub(1)
                    .and_then(|index| reviews.get(index)),
                &selection,
            )
            .map_err(store_error)
    }
}

fn store_error(error: impl std::fmt::Display) -> RepairEvidenceStoreError {
    RepairEvidenceStoreError(error.to_string())
}
