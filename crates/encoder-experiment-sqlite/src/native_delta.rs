use chrono::{DateTime, Utc};
use encoder_repair_core::{
    ports::{BoxFuture, NativeRepairQualityStore, RepairEvidenceStore, RepairEvidenceStoreError},
    quality::{
        ApprovedNativeDeltaSelection, NativeDeltaCandidateSet, NativeDeltaQualityReport,
        NativeDeltaReview,
    },
};
use uuid::Uuid;

use crate::SqliteExperimentStore;

#[derive(sqlx::FromRow)]
struct CandidateSetStorageRow {
    evidence_fingerprint: String,
    fingerprint: String,
    proposal_id: Uuid,
    application_id: Uuid,
    execution_project_id: Uuid,
    artifact_json: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ReportStorageRow {
    fingerprint: String,
    candidate_set_id: Uuid,
    proposal_id: Uuid,
    policy_fingerprint: String,
    eligible: bool,
    artifact_json: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct ReviewStorageRow {
    id: Uuid,
    fingerprint: String,
    proposal_id: Uuid,
    predecessor_id: Option<Uuid>,
    decision: String,
    artifact_json: String,
    created_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct SelectionStorageRow {
    specification_fingerprint: String,
    fingerprint: String,
    proposal_id: Uuid,
    candidate_set_id: Uuid,
    report_id: Uuid,
    approval_id: Uuid,
    artifact_json: String,
    created_at: DateTime<Utc>,
}

impl NativeRepairQualityStore for SqliteExperimentStore {
    fn create_native_delta_candidate_set(
        &self,
        candidate_set: NativeDeltaCandidateSet,
    ) -> BoxFuture<'_, Result<NativeDeltaCandidateSet, RepairEvidenceStoreError>> {
        Box::pin(async move {
            self.validate_candidate_set(&candidate_set).await?;
            if let Some(existing) = self
                .find_native_delta_candidate_set_by_evidence(
                    candidate_set.evidence_fingerprint.clone(),
                )
                .await?
            {
                return Ok(existing);
            }
            let artifact_json = serde_json::to_string(&candidate_set).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_native_delta_candidate_sets \
                 (id, evidence_fingerprint, fingerprint, proposal_id, application_id, \
                  execution_project_snapshot_id, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(candidate_set.id)
            .bind(&candidate_set.evidence_fingerprint)
            .bind(&candidate_set.fingerprint)
            .bind(candidate_set.proposal.id)
            .bind(candidate_set.application.id)
            .bind(candidate_set.execution_project_id)
            .bind(artifact_json)
            .bind(candidate_set.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(candidate_set)
        })
    }

    fn get_native_delta_candidate_set(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<NativeDeltaCandidateSet>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let stored: Option<CandidateSetStorageRow> = sqlx::query_as(
                "SELECT evidence_fingerprint, fingerprint, proposal_id, application_id, \
                 execution_project_snapshot_id AS execution_project_id, artifact_json, created_at \
                 FROM encoder_native_delta_candidate_sets WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(stored) = stored else {
                return Ok(None);
            };
            let value: NativeDeltaCandidateSet =
                serde_json::from_str(&stored.artifact_json).map_err(store_error)?;
            if value.id != id
                || value.evidence_fingerprint != stored.evidence_fingerprint
                || value.fingerprint != stored.fingerprint
                || value.proposal.id != stored.proposal_id
                || value.application.id != stored.application_id
                || value.execution_project_id != stored.execution_project_id
                || value.created_at != stored.created_at
            {
                return Err(RepairEvidenceStoreError(
                    "native delta candidate-set storage envelope changed".into(),
                ));
            }
            self.validate_candidate_set(&value).await?;
            Ok(Some(value))
        })
    }

    fn find_native_delta_candidate_set_by_evidence(
        &self,
        evidence_fingerprint: String,
    ) -> BoxFuture<'_, Result<Option<NativeDeltaCandidateSet>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_native_delta_candidate_sets WHERE evidence_fingerprint = ?",
            )
            .bind(evidence_fingerprint)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match id {
                Some(id) => self.get_native_delta_candidate_set(id).await,
                None => Ok(None),
            }
        })
    }

    fn create_native_delta_report(
        &self,
        report: NativeDeltaQualityReport,
    ) -> BoxFuture<'_, Result<NativeDeltaQualityReport, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let candidate_set = self
                .get_native_delta_candidate_set(report.candidate_set.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("native delta candidate set is missing".into())
                })?;
            let proposal = self
                .get_proposal(candidate_set.proposal.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("native delta proposal is missing".into())
                })?;
            report
                .validate_against(&proposal, &candidate_set)
                .map_err(store_error)?;
            if let Some(existing) = self
                .get_native_delta_report_for_candidate_set(candidate_set.id)
                .await?
            {
                return Ok(existing);
            }
            let artifact_json = serde_json::to_string(&report).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_native_delta_reports \
                 (id, fingerprint, candidate_set_id, proposal_id, policy_fingerprint, \
                  eligible, artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(report.id)
            .bind(&report.fingerprint)
            .bind(report.candidate_set.id)
            .bind(report.proposal.id)
            .bind(&report.policy_fingerprint)
            .bind(report.eligible)
            .bind(artifact_json)
            .bind(report.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(report)
        })
    }

    fn get_native_delta_report(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<NativeDeltaQualityReport>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let stored: Option<ReportStorageRow> = sqlx::query_as(
                "SELECT fingerprint, candidate_set_id, proposal_id, policy_fingerprint, \
                 eligible, artifact_json, created_at \
                 FROM encoder_native_delta_reports WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(stored) = stored else {
                return Ok(None);
            };
            let value: NativeDeltaQualityReport =
                serde_json::from_str(&stored.artifact_json).map_err(store_error)?;
            if value.id != id
                || value.fingerprint != stored.fingerprint
                || value.candidate_set.id != stored.candidate_set_id
                || value.proposal.id != stored.proposal_id
                || value.policy_fingerprint != stored.policy_fingerprint
                || value.eligible != stored.eligible
                || value.created_at != stored.created_at
            {
                return Err(RepairEvidenceStoreError(
                    "native delta report storage envelope changed".into(),
                ));
            }
            let candidate_set = self
                .get_native_delta_candidate_set(value.candidate_set.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("native delta candidate set disappeared".into())
                })?;
            let proposal = self.get_proposal(value.proposal.id).await?.ok_or_else(|| {
                RepairEvidenceStoreError("native delta proposal disappeared".into())
            })?;
            value
                .validate_against(&proposal, &candidate_set)
                .map_err(store_error)?;
            Ok(Some(value))
        })
    }

    fn get_native_delta_report_for_candidate_set(
        &self,
        candidate_set_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<NativeDeltaQualityReport>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_native_delta_reports WHERE candidate_set_id = ?",
            )
            .bind(candidate_set_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match id {
                Some(id) => self.get_native_delta_report(id).await,
                None => Ok(None),
            }
        })
    }

    fn append_native_delta_review(
        &self,
        review: NativeDeltaReview,
    ) -> BoxFuture<'_, Result<NativeDeltaReview, RepairEvidenceStoreError>> {
        Box::pin(async move {
            if self
                .get_native_delta_selection_for_proposal(review.proposal.id)
                .await?
                .is_some()
            {
                return Err(RepairEvidenceStoreError(
                    "native delta review chain is frozen by its approved selection".into(),
                ));
            }
            let report = self
                .get_native_delta_report(review.report.id)
                .await?
                .ok_or_else(|| RepairEvidenceStoreError("native delta report is missing".into()))?;
            let candidate_set = self
                .get_native_delta_candidate_set(report.candidate_set.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("native delta candidate set is missing".into())
                })?;
            let proposal = self
                .get_proposal(report.proposal.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("native delta proposal is missing".into())
                })?;
            let reviews = self.list_native_delta_reviews(report.id).await?;
            review
                .validate_against(&proposal, &candidate_set, &report, reviews.last())
                .map_err(store_error)?;
            let artifact_json = serde_json::to_string(&review).map_err(store_error)?;
            let decision = serde_json::to_string(&review.decision).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_native_delta_reviews \
                 (id, fingerprint, report_id, proposal_id, predecessor_id, decision, \
                  artifact_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(&review.fingerprint)
            .bind(review.report.id)
            .bind(review.proposal.id)
            .bind(review.predecessor.as_ref().map(|value| value.id))
            .bind(decision)
            .bind(artifact_json)
            .bind(review.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(review)
        })
    }

    fn list_native_delta_reviews(
        &self,
        report_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<NativeDeltaReview>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let report = self
                .get_native_delta_report(report_id)
                .await?
                .ok_or_else(|| RepairEvidenceStoreError("native delta report is missing".into()))?;
            let candidate_set = self
                .get_native_delta_candidate_set(report.candidate_set.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("native delta candidate set is missing".into())
                })?;
            let proposal = self
                .get_proposal(report.proposal.id)
                .await?
                .ok_or_else(|| {
                    RepairEvidenceStoreError("native delta proposal is missing".into())
                })?;
            let stored: Vec<ReviewStorageRow> = sqlx::query_as(
                "SELECT id, fingerprint, proposal_id, predecessor_id, decision, \
                 artifact_json, created_at FROM encoder_native_delta_reviews \
                 WHERE report_id = ? ORDER BY created_at, id",
            )
            .bind(report_id)
            .fetch_all(self.pool())
            .await
            .map_err(store_error)?;
            let mut values = Vec::with_capacity(stored.len());
            for stored in stored {
                let value: NativeDeltaReview =
                    serde_json::from_str(&stored.artifact_json).map_err(store_error)?;
                if value.id != stored.id
                    || value.fingerprint != stored.fingerprint
                    || value.report.id != report_id
                    || value.proposal.id != stored.proposal_id
                    || value.predecessor.as_ref().map(|binding| binding.id) != stored.predecessor_id
                    || serde_json::to_string(&value.decision).map_err(store_error)?
                        != stored.decision
                    || value.created_at != stored.created_at
                {
                    return Err(RepairEvidenceStoreError(
                        "native delta review storage envelope changed".into(),
                    ));
                }
                value
                    .validate_against(&proposal, &candidate_set, &report, values.last())
                    .map_err(store_error)?;
                values.push(value);
            }
            Ok(values)
        })
    }

    fn create_native_delta_selection(
        &self,
        selection: ApprovedNativeDeltaSelection,
    ) -> BoxFuture<'_, Result<ApprovedNativeDeltaSelection, RepairEvidenceStoreError>> {
        Box::pin(async move {
            self.validate_selection(&selection).await?;
            if let Some(existing) = self
                .get_native_delta_selection_for_proposal(selection.proposal.id)
                .await?
            {
                if existing.specification_fingerprint != selection.specification_fingerprint {
                    return Err(RepairEvidenceStoreError(
                        "native delta proposal already has a different approved selection".into(),
                    ));
                }
                return Ok(existing);
            }
            let artifact_json = serde_json::to_string(&selection).map_err(store_error)?;
            sqlx::query(
                "INSERT INTO encoder_native_delta_selections \
                 (id, specification_fingerprint, fingerprint, proposal_id, candidate_set_id, \
                  report_id, approval_id, artifact_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(selection.id)
            .bind(&selection.specification_fingerprint)
            .bind(&selection.fingerprint)
            .bind(selection.proposal.id)
            .bind(selection.candidate_set.id)
            .bind(selection.report.id)
            .bind(selection.approval.id)
            .bind(artifact_json)
            .bind(selection.created_at)
            .execute(self.pool())
            .await
            .map_err(store_error)?;
            Ok(selection)
        })
    }

    fn get_native_delta_selection(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ApprovedNativeDeltaSelection>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let stored: Option<SelectionStorageRow> = sqlx::query_as(
                "SELECT specification_fingerprint, fingerprint, proposal_id, candidate_set_id, \
                 report_id, approval_id, artifact_json, created_at \
                 FROM encoder_native_delta_selections WHERE id = ?",
            )
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            let Some(stored) = stored else {
                return Ok(None);
            };
            let value: ApprovedNativeDeltaSelection =
                serde_json::from_str(&stored.artifact_json).map_err(store_error)?;
            if value.id != id
                || value.specification_fingerprint != stored.specification_fingerprint
                || value.fingerprint != stored.fingerprint
                || value.proposal.id != stored.proposal_id
                || value.candidate_set.id != stored.candidate_set_id
                || value.report.id != stored.report_id
                || value.approval.id != stored.approval_id
                || value.created_at != stored.created_at
            {
                return Err(RepairEvidenceStoreError(
                    "native delta selection storage envelope changed".into(),
                ));
            }
            self.validate_selection(&value).await?;
            Ok(Some(value))
        })
    }

    fn get_native_delta_selection_for_proposal(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ApprovedNativeDeltaSelection>, RepairEvidenceStoreError>> {
        Box::pin(async move {
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM encoder_native_delta_selections WHERE proposal_id = ?",
            )
            .bind(proposal_id)
            .fetch_optional(self.pool())
            .await
            .map_err(store_error)?;
            match id {
                Some(id) => self.get_native_delta_selection(id).await,
                None => Ok(None),
            }
        })
    }
}

impl SqliteExperimentStore {
    async fn validate_candidate_set(
        &self,
        candidate_set: &NativeDeltaCandidateSet,
    ) -> Result<(), RepairEvidenceStoreError> {
        let proposal = self
            .get_proposal(candidate_set.proposal.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("native delta proposal is missing".into()))?;
        let application = self
            .get_proposal_application(proposal.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("repair application is missing".into()))?;
        if candidate_set.application.id != application.id
            || candidate_set.application.fingerprint != application.fingerprint
            || candidate_set.created_at < application.created_at
        {
            return Err(RepairEvidenceStoreError(
                "native delta does not bind the exact repair application".into(),
            ));
        }
        candidate_set
            .validate_against(&proposal)
            .map_err(store_error)
    }

    async fn validate_selection(
        &self,
        selection: &ApprovedNativeDeltaSelection,
    ) -> Result<(), RepairEvidenceStoreError> {
        let proposal = self
            .get_proposal(selection.proposal.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("native delta proposal is missing".into()))?;
        let candidate_set = self
            .get_native_delta_candidate_set(selection.candidate_set.id)
            .await?
            .ok_or_else(|| {
                RepairEvidenceStoreError("native delta candidate set is missing".into())
            })?;
        let report = self
            .get_native_delta_report(selection.report.id)
            .await?
            .ok_or_else(|| RepairEvidenceStoreError("native delta report is missing".into()))?;
        let reviews = self.list_native_delta_reviews(report.id).await?;
        let approval_index = reviews
            .iter()
            .position(|value| value.id == selection.approval.id)
            .ok_or_else(|| RepairEvidenceStoreError("native delta approval is missing".into()))?;
        let approval = &reviews[approval_index];
        if approval_index + 1 != reviews.len() {
            return Err(RepairEvidenceStoreError(
                "native delta selection does not bind the latest review".into(),
            ));
        }
        selection
            .validate_against(
                &proposal,
                &candidate_set,
                &report,
                approval,
                approval_index
                    .checked_sub(1)
                    .and_then(|index| reviews.get(index)),
            )
            .map_err(store_error)
    }
}

fn store_error(error: impl std::fmt::Display) -> RepairEvidenceStoreError {
    RepairEvidenceStoreError(error.to_string())
}
