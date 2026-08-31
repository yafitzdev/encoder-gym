//! SQLite persistence for dataset qualification and curation.

use std::collections::{BTreeMap, BTreeSet};

use dataset_core::domain::{DatasetSnapshot, SnapshotMember, SourceRow};
use dataset_quality_core::{
    QualityError,
    assessment::{BlindEvaluatorRequest, EvaluatorGuidance, QualityVerdict, RowQualityAssessment},
    curation::{
        ApprovedCurationManifest, ArtifactReference, CurationApplication, CurationDecision,
        CurationDecisionBasis, CurationDisposition, CurationManifestReview,
        CurationManifestReviewDecision, CurationProposal, DatasetQualityReport,
        DatasetQualityReportRow, ReportRowVerdict, RowQualityReview, RowQualityReviewDecision,
    },
    lifecycle::{
        AuditExecutionLease, AuditPlanStatusProjection, AuditProgress, AuditStatusProjection,
        AuditUsage, EvaluatorAttempt, EvaluatorAttemptState, FinishedAttemptEvidence,
        QualityAuditRun, QualityAuditRunState,
    },
    population::{
        AUDIT_PLAN_SCHEMA_VERSION, AuditPlan, AuditPlanItem, AuditSelection, CheckedAuditPlan,
    },
    ports::{BoxFuture, DatasetQualityStore, QualityAdapterError, QualityCandidateSource},
};
use sqlx::{FromRow, QueryBuilder, Row, Sqlite, SqliteConnection};
use sysinfo::{Pid, System};
use uuid::Uuid;
use workflow_core::governance::{
    CohortDisposition, CohortOrigin, CohortRole, CohortRoleDecision, EvaluationCohort,
};

use crate::{SnapshotMemberRecord, SnapshotRecord, SourceRowRecord, SqliteStore, insert_snapshot};

impl QualityCandidateSource for SqliteStore {
    fn list_source_rows(
        &self,
        dataset_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<SourceRow>, QualityAdapterError>> {
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            validate_quality_candidate_governance_in(&mut transaction).await?;
            let rows = sqlx::query_as::<_, SourceRowRecord>(
                "SELECT id, dataset_id, text, label, dimensions_json, fields_json, \
                 provenance_json, created_at FROM dataset_source_rows AS candidate \
                 WHERE dataset_id = ? AND NOT EXISTS (\
                     SELECT 1 FROM workflow_evaluation_cohorts AS cohort \
                      JOIN dataset_snapshot_members AS member \
                       ON member.snapshot_id = cohort.snapshot_id \
                      AND member.split = cohort.split \
                      AND member.source_row_id = candidate.id \
                     WHERE cohort.origin = 'external_benchmark' \
                        OR EXISTS (\
                            SELECT 1 FROM workflow_cohort_role_decisions AS historical_role \
                            WHERE historical_role.cohort_id = cohort.id \
                              AND historical_role.role IN ('sealed_acceptance', 'external_benchmark')\
                        ) \
                        OR COALESCE((\
                            SELECT current_role.role \
                            FROM workflow_cohort_role_decisions AS current_role \
                            WHERE current_role.cohort_id = cohort.id \
                            ORDER BY current_role.sequence DESC LIMIT 1\
                        ), '') != 'training' \
                 ) ORDER BY id",
            )
            .bind(dataset_id)
            .fetch_all(&mut *transaction)
            .await
            .map_err(sql_error)?;
            let rows = decode_source_rows(rows)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(rows)
        })
    }

    fn get_source_rows(
        &self,
        dataset_id: Uuid,
        source_row_ids: Vec<Uuid>,
    ) -> BoxFuture<'_, Result<Vec<SourceRow>, QualityAdapterError>> {
        Box::pin(async move {
            let requested = canonical_requested_ids(source_row_ids)?;
            if requested.is_empty() {
                return Ok(Vec::new());
            }
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            validate_quality_candidate_governance_in(&mut transaction).await?;
            let mut builder = QueryBuilder::<Sqlite>::new(
                "SELECT id, dataset_id, text, label, dimensions_json, fields_json, \
                 provenance_json, created_at FROM dataset_source_rows AS candidate \
                 WHERE dataset_id = ",
            );
            builder.push_bind(dataset_id).push(" AND id IN (");
            let mut separated = builder.separated(", ");
            for id in &requested {
                separated.push_bind(id);
            }
            separated.push_unseparated(
                ") AND NOT EXISTS (\
                    SELECT 1 FROM workflow_evaluation_cohorts AS cohort \
                    JOIN dataset_snapshot_members AS member \
                      ON member.snapshot_id = cohort.snapshot_id \
                     AND member.split = cohort.split \
                     AND member.source_row_id = candidate.id \
                    WHERE cohort.origin = 'external_benchmark' \
                       OR EXISTS (\
                           SELECT 1 FROM workflow_cohort_role_decisions AS historical_role \
                           WHERE historical_role.cohort_id = cohort.id \
                             AND historical_role.role IN ('sealed_acceptance', 'external_benchmark')\
                       ) \
                       OR COALESCE((\
                           SELECT current_role.role \
                           FROM workflow_cohort_role_decisions AS current_role \
                           WHERE current_role.cohort_id = cohort.id \
                           ORDER BY current_role.sequence DESC LIMIT 1\
                       ), '') != 'training' \
                ) ORDER BY id",
            );
            let rows = builder
                .build_query_as::<SourceRowRecord>()
                .fetch_all(&mut *transaction)
                .await
                .map_err(sql_error)?;
            let rows = decode_source_rows(rows)?;
            require_exact_source_ids(&requested, &rows)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(rows)
        })
    }
}

impl DatasetQualityStore for SqliteStore {
    fn create_audit(
        &self,
        plan: &AuditPlan,
        run: &QualityAuditRun,
        guidance: &EvaluatorGuidance,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let plan = plan.clone();
        let run = run.clone();
        let guidance = guidance.clone();
        Box::pin(async move {
            validate_plan_run_creation(&plan, &run, &guidance)?;
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            verify_plan_sources_in(connection, &plan).await?;
            insert_plan_in(connection, &plan).await?;
            insert_guidance_in(connection, &plan, &guidance).await?;
            insert_run_in(connection, &run).await?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_audit_plan(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AuditPlan>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_plan_in(&mut connection, id).await
        })
    }

    fn get_audit_guidance(
        &self,
        plan_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<EvaluatorGuidance>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            let Some(plan) = load_plan_in(&mut connection, plan_id).await? else {
                return Ok(None);
            };
            load_guidance_in(&mut connection, &plan).await
        })
    }

    fn get_audit_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<QualityAuditRun>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            let Some(run) = load_run_in(&mut connection, id).await? else {
                return Ok(None);
            };
            let plan = load_plan_in(&mut connection, run.plan_id)
                .await?
                .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
            let requests = load_requests_for_run_in(&mut connection, run.id).await?;
            let attempts = load_attempts_for_run_in(&mut connection, run.id).await?;
            let assessments = load_assessments_for_run_in(&mut connection, run.id).await?;
            require_assessment_count(&mut connection, run.id, assessments.len()).await?;
            run.verify_against_evidence(&plan, &requests, &attempts, &assessments)
                .map_err(domain_error)?;
            Ok(Some(run))
        })
    }

    fn audit_cancel_requested(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<bool>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            Ok(load_run_projection_in(&mut connection, id)
                .await?
                .map(|(run, _)| run.cancel_requested))
        })
    }

    fn get_audit_status(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<AuditStatusProjection>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_audit_status_in(&mut connection, id).await
        })
    }

    fn save_audit_run(
        &self,
        run: &QualityAuditRun,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let run = run.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            let plan = load_plan_in(connection, run.plan_id)
                .await?
                .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
            validate_run(&plan, &run)?;
            let previous = load_run_in(connection, run.id)
                .await?
                .ok_or_else(|| adapter_error("quality audit run is missing"))?;
            let requests = load_requests_for_run_in(connection, run.id).await?;
            let attempts = load_attempts_for_run_in(connection, run.id).await?;
            let assessments = load_assessments_for_run_in(connection, run.id).await?;
            require_assessment_count(connection, run.id, assessments.len()).await?;
            run.verify_against_evidence(&plan, &requests, &attempts, &assessments)
                .map_err(domain_error)?;
            if previous == run {
                transaction.commit().await.map_err(sql_error)?;
                return Ok(());
            }
            validate_run_transition(&previous, &run)?;
            update_run_in(connection, &previous, &run).await?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn acquire_audit_execution_lease(
        &self,
        run_id: Uuid,
        invocation_token: Uuid,
    ) -> BoxFuture<'_, Result<AuditExecutionLease, QualityAdapterError>> {
        Box::pin(async move {
            if invocation_token.is_nil() {
                return Err(adapter_error(
                    "audit execution invocation token must not be nil",
                ));
            }
            let process_id = std::process::id();
            let process_started_at = quality_process_started_at(process_id)?;
            let lease = AuditExecutionLease::create(
                run_id,
                invocation_token,
                process_id,
                process_started_at,
                chrono::Utc::now(),
            )
            .map_err(domain_error)?;
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            if load_run_projection_in(connection, run_id).await?.is_none() {
                return Err(adapter_error("quality audit run is missing"));
            }
            let inserted = insert_execution_lease_in(connection, &lease).await?;
            if !inserted {
                let previous = load_execution_lease_in(connection, run_id)
                    .await?
                    .ok_or_else(|| adapter_error("quality audit lease changed concurrently"))?;
                if execution_lease_owner_is_active(&previous) {
                    return Err(adapter_error(format!(
                        "quality audit {run_id} is owned by another live execution"
                    )));
                }
                replace_execution_lease_in(connection, &previous, &lease).await?;
            }
            transaction.commit().await.map_err(sql_error)?;
            Ok(lease)
        })
    }

    fn release_audit_execution_lease(
        &self,
        lease: &AuditExecutionLease,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let lease = lease.clone();
        Box::pin(async move {
            lease.verify().map_err(domain_error)?;
            let result = sqlx::query(
                "DELETE FROM dataset_quality_execution_leases WHERE run_id = ? \
                 AND invocation_token = ? AND process_id = ? AND process_started_at = ? \
                 AND acquired_at = ?",
            )
            .bind(lease.run_id)
            .bind(lease.invocation_token)
            .bind(i64::from(lease.process_id))
            .bind(as_i64(lease.process_started_at)?)
            .bind(lease.acquired_at)
            .execute(self.pool())
            .await
            .map_err(sql_error)?;
            require_one(result.rows_affected(), "quality audit execution lease")
        })
    }

    fn record_attempt<'a>(
        &'a self,
        checked: &'a CheckedAuditPlan<'a>,
        lease: &'a AuditExecutionLease,
        attempt: &'a EvaluatorAttempt,
        request: &'a BlindEvaluatorRequest,
        run: &'a QualityAuditRun,
    ) -> BoxFuture<'a, Result<(), QualityAdapterError>> {
        Box::pin(async move {
            record_attempt_with_context_in(self, checked, lease, attempt, request, run).await
        })
    }

    fn get_evaluator_request(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BlindEvaluatorRequest>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_request_in(&mut connection, id).await
        })
    }

    fn list_evaluator_requests(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BlindEvaluatorRequest>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_requests_for_run_in(&mut connection, run_id).await
        })
    }

    fn finish_attempt<'a>(
        &'a self,
        checked: &'a CheckedAuditPlan<'a>,
        lease: &'a AuditExecutionLease,
        attempt: &'a EvaluatorAttempt,
        assessments: &'a [RowQualityAssessment],
        run: &'a QualityAuditRun,
    ) -> BoxFuture<'a, Result<(), QualityAdapterError>> {
        Box::pin(async move {
            finish_attempt_with_context_in(self, checked, lease, attempt, assessments, run).await
        })
    }

    fn list_attempts(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<EvaluatorAttempt>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_attempts_for_run_in(&mut connection, run_id).await
        })
    }

    fn list_assessments(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RowQualityAssessment>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_assessments_for_run_in(&mut connection, run_id).await
        })
    }

    fn get_assessment(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<RowQualityAssessment>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_assessment_in(&mut connection, id).await
        })
    }

    fn save_report(
        &self,
        report: &DatasetQualityReport,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let report = report.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            validate_report_in(connection, &report).await?;
            if let Some(existing) = load_report_in(connection, report.id).await? {
                if existing == report {
                    transaction.commit().await.map_err(sql_error)?;
                    return Ok(());
                }
                return Err(adapter_error("dataset quality report identity collision"));
            }
            insert_report_in(connection, &report).await?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_report(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetQualityReport>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_report_in(&mut connection, id).await
        })
    }

    fn report_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<DatasetQualityReport>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            let id: Option<Uuid> =
                sqlx::query_scalar("SELECT id FROM dataset_quality_reports WHERE run_id = ?")
                    .bind(run_id)
                    .fetch_optional(&mut *connection)
                    .await
                    .map_err(sql_error)?;
            match id {
                Some(id) => load_report_in(&mut connection, id).await,
                None => Ok(None),
            }
        })
    }

    fn append_row_review(
        &self,
        review: &RowQualityReview,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let review = review.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            if let Some(existing) = load_row_review_by_id_in(connection, review.id).await? {
                if existing == review {
                    transaction.commit().await.map_err(sql_error)?;
                    return Ok(());
                }
                return Err(adapter_error("row review identity collision"));
            }
            let report = load_report_in(connection, review.report_id)
                .await?
                .ok_or_else(|| adapter_error("dataset quality report is missing"))?;
            validate_row_review(&report, &review)?;
            let latest =
                latest_row_review_in(connection, review.report_id, review.source_row_id).await?;
            require_review_predecessor(
                review.predecessor_id,
                review.predecessor_fingerprint.as_deref(),
                latest
                    .as_ref()
                    .map(|value| (value.id, value.fingerprint.as_str())),
                "row review",
            )?;
            sqlx::query(
                "INSERT INTO dataset_quality_row_reviews \
                 (id, report_id, source_row_id, predecessor_id, decision, fingerprint, \
                  review_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.report_id)
            .bind(review.source_row_id)
            .bind(review.predecessor_id)
            .bind(row_review_decision(review.decision))
            .bind(&review.fingerprint)
            .bind(encode(&review)?)
            .bind(review.created_at)
            .execute(connection)
            .await
            .map_err(sql_error)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_row_reviews(
        &self,
        report_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<RowQualityReview>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_row_reviews_in(&mut connection, report_id).await
        })
    }

    fn save_curation_proposal(
        &self,
        proposal: &CurationProposal,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let proposal = proposal.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            validate_proposal_in(connection, &proposal).await?;
            if let Some(existing) = load_proposal_in(connection, proposal.id).await? {
                if existing == proposal {
                    transaction.commit().await.map_err(sql_error)?;
                    return Ok(());
                }
                return Err(adapter_error("curation proposal identity collision"));
            }
            insert_proposal_in(connection, &proposal).await?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_curation_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationProposal>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_proposal_in(&mut connection, id).await
        })
    }

    fn latest_curation_proposal(
        &self,
        report_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationProposal>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM dataset_curation_proposals WHERE report_id = ? \
                 ORDER BY rowid DESC LIMIT 1",
            )
            .bind(report_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
            match id {
                Some(id) => load_proposal_in(&mut connection, id).await,
                None => Ok(None),
            }
        })
    }

    fn append_manifest_review(
        &self,
        review: &CurationManifestReview,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let review = review.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            if let Some(existing) = load_manifest_review_by_id_in(connection, review.id).await? {
                if existing == review {
                    transaction.commit().await.map_err(sql_error)?;
                    return Ok(());
                }
                return Err(adapter_error("manifest review identity collision"));
            }
            let proposal = load_proposal_in(connection, review.proposal_id)
                .await?
                .ok_or_else(|| adapter_error("curation proposal is missing"))?;
            validate_manifest_review(&proposal, &review)?;
            let sealed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM dataset_curation_manifests WHERE proposal_id = ?)",
            )
            .bind(review.proposal_id)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
            if sealed {
                return Err(adapter_error(
                    "an approved curation manifest seals its manifest-review chain",
                ));
            }
            let latest = latest_manifest_review_in(connection, review.proposal_id).await?;
            require_review_predecessor(
                review.predecessor_id,
                review.predecessor_fingerprint.as_deref(),
                latest
                    .as_ref()
                    .map(|value| (value.id, value.fingerprint.as_str())),
                "manifest review",
            )?;
            sqlx::query(
                "INSERT INTO dataset_curation_manifest_reviews \
                 (id, proposal_id, predecessor_id, decision, fingerprint, review_json, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(review.id)
            .bind(review.proposal_id)
            .bind(review.predecessor_id)
            .bind(manifest_review_decision(review.decision))
            .bind(&review.fingerprint)
            .bind(encode(&review)?)
            .bind(review.created_at)
            .execute(connection)
            .await
            .map_err(sql_error)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn list_manifest_reviews(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<CurationManifestReview>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_manifest_reviews_in(&mut connection, proposal_id).await
        })
    }

    fn save_manifest(
        &self,
        manifest: &ApprovedCurationManifest,
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let manifest = manifest.clone();
        Box::pin(async move {
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;
            validate_manifest_in(connection, &manifest).await?;
            if let Some(existing) = load_manifest_in(connection, manifest.id).await? {
                if existing == manifest {
                    transaction.commit().await.map_err(sql_error)?;
                    return Ok(());
                }
                return Err(adapter_error("curation manifest identity collision"));
            }
            insert_manifest_in(connection, &manifest).await?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn get_manifest(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ApprovedCurationManifest>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            load_manifest_in(&mut connection, id).await
        })
    }

    fn manifest_for_proposal(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ApprovedCurationManifest>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            let id: Option<Uuid> = sqlx::query_scalar(
                "SELECT id FROM dataset_curation_manifests WHERE proposal_id = ?",
            )
            .bind(proposal_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
            match id {
                Some(id) => load_manifest_in(&mut connection, id).await,
                None => Ok(None),
            }
        })
    }

    fn apply_manifest(
        &self,
        application: &CurationApplication,
        snapshot: &DatasetSnapshot,
        members: &[SnapshotMember],
    ) -> BoxFuture<'_, Result<(), QualityAdapterError>> {
        let application = application.clone();
        let snapshot = snapshot.clone();
        let members = members.to_vec();
        Box::pin(async move {
            validate_snapshot(&snapshot, &members)?;
            let mut transaction = self.pool().begin().await.map_err(sql_error)?;
            let connection: &mut SqliteConnection = &mut transaction;

            if let Some(existing) = load_application_by_any_identity_in(
                connection,
                application.id,
                application.manifest_id,
                application.snapshot_id,
            )
            .await?
            {
                if existing != application {
                    return Err(adapter_error("curation application identity collision"));
                }
                validate_persisted_application_in(connection, &existing).await?;
                let (stored_snapshot, stored_members) =
                    load_snapshot_bundle_in(connection, existing.snapshot_id).await?;
                if stored_snapshot != snapshot || stored_members != members {
                    return Err(adapter_error(
                        "idempotent curation application replay differs from the stored snapshot",
                    ));
                }
                transaction.commit().await.map_err(sql_error)?;
                return Ok(());
            }

            let manifest = load_manifest_in(connection, application.manifest_id)
                .await?
                .ok_or_else(|| adapter_error("approved curation manifest is missing"))?;
            require_current_manifest_for_new_application_in(connection, &manifest).await?;
            validate_application_in(connection, &application, &manifest, &snapshot, &members)
                .await?;
            verify_snapshot_sources_in(connection, &manifest, &members).await?;

            insert_snapshot(connection, &snapshot, &members)
                .await
                .map_err(|error| adapter_error(error.to_string()))?;
            sqlx::query(
                "INSERT INTO dataset_curation_applications \
                 (id, manifest_id, approval_id, snapshot_id, manifest_fingerprint, \
                  snapshot_fingerprint, selected_member_fingerprint, \
                  snapshot_membership_fingerprint, fingerprint, application_json, applied_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(application.id)
            .bind(application.manifest_id)
            .bind(application.approval_id)
            .bind(application.snapshot_id)
            .bind(&application.manifest_fingerprint)
            .bind(&application.snapshot_fingerprint)
            .bind(&application.selected_member_fingerprint)
            .bind(&application.snapshot_membership_fingerprint)
            .bind(&application.fingerprint)
            .bind(encode(&application)?)
            .bind(application.applied_at)
            .execute(connection)
            .await
            .map_err(sql_error)?;
            transaction.commit().await.map_err(sql_error)?;
            Ok(())
        })
    }

    fn curation_application_for_snapshot(
        &self,
        snapshot_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationApplication>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            let record = sqlx::query_as::<_, ApplicationRecord>(
                "SELECT id, manifest_id, approval_id, snapshot_id, manifest_fingerprint, \
                 snapshot_fingerprint, selected_member_fingerprint, \
                 snapshot_membership_fingerprint, fingerprint, application_json, applied_at \
                 FROM dataset_curation_applications WHERE snapshot_id = ?",
            )
            .bind(snapshot_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
            let Some(record) = record else {
                return Ok(None);
            };
            let value = check_application_record(record)?;
            validate_persisted_application_in(&mut connection, &value).await?;
            Ok(Some(value))
        })
    }

    fn curation_application_for_manifest(
        &self,
        manifest_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<CurationApplication>, QualityAdapterError>> {
        Box::pin(async move {
            let mut connection = self.pool().acquire().await.map_err(sql_error)?;
            let record = sqlx::query_as::<_, ApplicationRecord>(
                "SELECT id, manifest_id, approval_id, snapshot_id, manifest_fingerprint, \
                 snapshot_fingerprint, selected_member_fingerprint, \
                 snapshot_membership_fingerprint, fingerprint, application_json, applied_at \
                 FROM dataset_curation_applications WHERE manifest_id = ?",
            )
            .bind(manifest_id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
            let Some(record) = record else {
                return Ok(None);
            };
            let value = check_application_record(record)?;
            validate_persisted_application_in(&mut connection, &value).await?;
            Ok(Some(value))
        })
    }
}

async fn record_attempt_with_context_in(
    store: &SqliteStore,
    checked: &CheckedAuditPlan<'_>,
    lease: &AuditExecutionLease,
    attempt: &EvaluatorAttempt,
    request: &BlindEvaluatorRequest,
    run: &QualityAuditRun,
) -> Result<(), QualityAdapterError> {
    if attempt.state != EvaluatorAttemptState::Started
        || run.state != QualityAuditRunState::Running
        || run.cancel_requested
    {
        return Err(adapter_error(
            "record_attempt accepts only a newly started attempt on a live audit run",
        ));
    }
    let mut transaction = store.pool().begin().await.map_err(sql_error)?;
    let connection: &mut SqliteConnection = &mut transaction;
    require_execution_lease_in(connection, lease).await?;
    require_persisted_checked_plan_in(connection, checked).await?;
    let (previous_run, _) = load_run_projection_in(connection, attempt.run_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit run is missing"))?;
    previous_run
        .verify_with_context(checked)
        .map_err(domain_error)?;

    if let Some((existing_request, existing_attempt)) =
        load_attempt_against_in(connection, attempt.id, checked, &previous_run).await?
    {
        if existing_attempt == *attempt && existing_request == *request && previous_run == *run {
            transaction.commit().await.map_err(sql_error)?;
            return Ok(());
        }
        return Err(adapter_error("evaluator attempt identity collision"));
    }

    run.verify_with_context(checked).map_err(domain_error)?;
    validate_run_timestamps(run)?;
    validate_run_transition(&previous_run, run)?;
    validate_attempt_with_context(checked, run, request, attempt)?;
    validate_attempt_reservation_with_context_in(
        connection,
        checked,
        &previous_run,
        request,
        attempt,
    )
    .await?;
    let mut expected_run = previous_run.clone();
    expected_run
        .reserve_attempt_with_context(checked, request, &attempt.evaluator)
        .map_err(domain_error)?;
    if expected_run != *run {
        return Err(adapter_error(
            "attempt reservation run does not reproduce from its exact delta",
        ));
    }
    insert_attempt_in(connection, request, attempt).await?;
    update_run_in(connection, &previous_run, run).await?;
    transaction.commit().await.map_err(sql_error)?;
    Ok(())
}

async fn finish_attempt_with_context_in(
    store: &SqliteStore,
    checked: &CheckedAuditPlan<'_>,
    lease: &AuditExecutionLease,
    attempt: &EvaluatorAttempt,
    assessments: &[RowQualityAssessment],
    run: &QualityAuditRun,
) -> Result<(), QualityAdapterError> {
    let mut transaction = store.pool().begin().await.map_err(sql_error)?;
    let connection: &mut SqliteConnection = &mut transaction;
    require_execution_lease_in(connection, lease).await?;
    require_persisted_checked_plan_in(connection, checked).await?;
    let (previous_run, _) = load_run_projection_in(connection, run.id)
        .await?
        .ok_or_else(|| adapter_error("quality audit run is missing"))?;
    previous_run
        .verify_with_context(checked)
        .map_err(domain_error)?;
    let (request, previous_attempt) =
        load_attempt_against_in(connection, attempt.id, checked, &previous_run)
            .await?
            .ok_or_else(|| adapter_error("started evaluator attempt is missing"))?;

    if previous_attempt == *attempt {
        let stored = load_assessments_for_attempt_against_in(
            connection,
            checked,
            &previous_run,
            &previous_attempt,
            &request,
        )
        .await?;
        if previous_run == *run
            && canonical_assessments(stored) == canonical_assessments(assessments.to_vec())
        {
            transaction.commit().await.map_err(sql_error)?;
            return Ok(());
        }
        return Err(adapter_error(
            "terminal evaluator attempt replay differs from storage",
        ));
    }

    validate_attempt_finish_with_context(
        checked,
        &previous_run,
        &request,
        &previous_attempt,
        attempt,
    )?;
    validate_finished_assessments_with_context(checked, &request, attempt, assessments)?;
    run.verify_with_context(checked).map_err(domain_error)?;
    validate_run_timestamps(run)?;
    validate_run_transition(&previous_run, run)?;
    let (prior_assessments, prior_invalid_attempts) =
        load_checked_row_local_result_evidence_with_context_in(
            connection,
            checked,
            &previous_run,
            &attempt.source_row_ids,
        )
        .await?;
    let mut expected_run = previous_run.clone();
    let overrun = expected_run
        .reconcile_finished_attempt_with_context(
            checked,
            FinishedAttemptEvidence {
                request: &request,
                previous_attempt: &previous_attempt,
                finished_attempt: attempt,
                prior_assessments: &prior_assessments,
                prior_invalid_attempts: &prior_invalid_attempts,
                new_assessments: assessments,
            },
        )
        .map_err(domain_error)?;
    if expected_run.progress != run.progress || expected_run.usage != run.usage {
        return Err(adapter_error(
            "finished attempt run counters do not reproduce from its exact row-local delta",
        ));
    }
    if overrun
        && !(run.state == QualityAuditRunState::Failed
            && run.stop_reason
                == Some(dataset_quality_core::lifecycle::QualityAuditStopReason::BudgetExhausted))
    {
        return Err(adapter_error(
            "attempt budget overrun requires a budget-exhausted terminal run",
        ));
    }

    for assessment in assessments {
        insert_assessment_in(connection, run.id, assessment).await?;
    }
    increment_assessment_count_in(connection, run.id, assessments.len()).await?;
    update_attempt_in(connection, &previous_attempt, attempt).await?;
    update_run_in(connection, &previous_run, run).await?;
    transaction.commit().await.map_err(sql_error)?;
    Ok(())
}

#[derive(Debug, FromRow)]
struct PlanRecord {
    id: Uuid,
    dataset_id: Uuid,
    dataset_fingerprint: String,
    policy_fingerprint: String,
    source_set_fingerprint: String,
    resolved_guidance_fingerprint: String,
    evaluator_protocol_version: String,
    fingerprint: String,
    plan_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct PlanStatusRecord {
    id: Uuid,
    dataset_id: Uuid,
    dataset_fingerprint: String,
    policy_fingerprint: String,
    source_set_fingerprint: String,
    resolved_guidance_fingerprint: String,
    evaluator_protocol_version: String,
    fingerprint: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct PlanItemRecord {
    source_row_id: Uuid,
    source_row_fingerprint: String,
    cell_json: String,
    provenance_stratum_json: String,
    selection: String,
    item_json: String,
}

#[derive(Debug, FromRow)]
struct RunRecord {
    id: Uuid,
    plan_id: Uuid,
    state: String,
    specification_fingerprint: String,
    population_rows: i64,
    selected_rows: i64,
    assessed_rows: i64,
    pending_review_rows: i64,
    qualified_rows: i64,
    borderline_rows: i64,
    quarantined_rows: i64,
    invalid_rows: i64,
    assessment_count: i64,
    evaluator_requests: i64,
    request_attempts: i64,
    input_tokens: i64,
    output_tokens: i64,
    total_tokens: i64,
    cost_microusd: i64,
    cancel_requested: bool,
    run_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, FromRow)]
struct ExecutionLeaseRecord {
    run_id: Uuid,
    invocation_token: Uuid,
    process_id: i64,
    process_started_at: i64,
    acquired_at: chrono::DateTime<chrono::Utc>,
}

impl ExecutionLeaseRecord {
    fn into_domain(self) -> Result<AuditExecutionLease, QualityAdapterError> {
        AuditExecutionLease::create(
            self.run_id,
            self.invocation_token,
            as_u32(self.process_id)?,
            as_u64(self.process_started_at)?,
            self.acquired_at,
        )
        .map_err(domain_error)
    }
}

#[derive(Debug, FromRow)]
struct AttemptRecord {
    id: Uuid,
    run_id: Uuid,
    request_id: Uuid,
    request_sequence: i64,
    attempt_number: i64,
    request_fingerprint: String,
    retry_payload_fingerprint: String,
    state: String,
    fingerprint: String,
    request_json: String,
    attempt_json: String,
    started_at: chrono::DateTime<chrono::Utc>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, FromRow)]
struct AssessmentRecord {
    id: Uuid,
    run_id: Uuid,
    plan_id: Uuid,
    attempt_id: Uuid,
    request_id: Uuid,
    source_row_id: Uuid,
    verdict: String,
    fingerprint: String,
    assessment_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ReportRecord {
    id: Uuid,
    run_id: Uuid,
    plan_id: Uuid,
    dataset_id: Uuid,
    assessment_set_fingerprint: String,
    invalid_attempt_set_fingerprint: String,
    fingerprint: String,
    report_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct RowReviewRecord {
    id: Uuid,
    report_id: Uuid,
    source_row_id: Uuid,
    predecessor_id: Option<Uuid>,
    decision: String,
    fingerprint: String,
    review_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ProposalRecord {
    id: Uuid,
    report_id: Uuid,
    dataset_id: Uuid,
    predecessor_id: Option<Uuid>,
    row_review_set_fingerprint: String,
    fingerprint: String,
    proposal_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ManifestReviewRecord {
    id: Uuid,
    proposal_id: Uuid,
    predecessor_id: Option<Uuid>,
    decision: String,
    fingerprint: String,
    review_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ManifestRecord {
    id: Uuid,
    report_id: Uuid,
    proposal_id: Uuid,
    approval_id: Uuid,
    dataset_id: Uuid,
    complete_member_fingerprint: String,
    selected_member_fingerprint: String,
    fingerprint: String,
    manifest_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct ApplicationRecord {
    id: Uuid,
    manifest_id: Uuid,
    approval_id: Uuid,
    snapshot_id: Uuid,
    manifest_fingerprint: String,
    snapshot_fingerprint: String,
    selected_member_fingerprint: String,
    snapshot_membership_fingerprint: String,
    fingerprint: String,
    application_json: String,
    applied_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, FromRow)]
struct QualityCandidateCohortRecord {
    id: Uuid,
    snapshot_id: Uuid,
    split: String,
    origin: String,
    name: String,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
    persisted_snapshot_fingerprint: Option<String>,
}

#[derive(Debug, FromRow)]
struct QualityCandidateRoleRecord {
    id: Uuid,
    cohort_id: Uuid,
    sequence: i64,
    role: String,
    disposition: String,
    predecessor_id: Option<Uuid>,
    fingerprint: String,
    artifact_json: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Verifies governance before any candidate text is selected. Keeping these as
/// two bounded set queries avoids a row-by-row governance lookup while making
/// normalized-column or append-only-history tampering fail closed.
async fn validate_quality_candidate_governance_in(
    connection: &mut SqliteConnection,
) -> Result<(), QualityAdapterError> {
    let cohort_records = sqlx::query_as::<_, QualityCandidateCohortRecord>(
        "SELECT cohort.id, cohort.snapshot_id, cohort.split, cohort.origin, cohort.name, \
         cohort.fingerprint, cohort.artifact_json, cohort.created_at, \
         snapshot.fingerprint AS persisted_snapshot_fingerprint \
         FROM workflow_evaluation_cohorts AS cohort \
         LEFT JOIN dataset_snapshots AS snapshot ON snapshot.id = cohort.snapshot_id \
         ORDER BY cohort.id",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut cohorts = BTreeMap::new();
    for record in cohort_records {
        let cohort: EvaluationCohort = decode(&record.artifact_json)?;
        if record.id != cohort.id
            || record.snapshot_id != cohort.snapshot_id
            || record.split != cohort.split.as_str()
            || record.origin != cohort_origin(cohort.origin)
            || record.name != cohort.name
            || record.fingerprint != cohort.fingerprint
            || record.created_at != cohort.created_at
            || record.persisted_snapshot_fingerprint.as_deref()
                != Some(cohort.snapshot_fingerprint.as_str())
            || cohort
                .reproduce_fingerprint()
                .map_err(|error| adapter_error(error.to_string()))?
                != cohort.fingerprint
        {
            return Err(adapter_error(
                "quality candidate cohort governance failed integrity validation",
            ));
        }
        if cohorts.insert(cohort.id, cohort).is_some() {
            return Err(adapter_error(
                "quality candidate cohort governance repeats an identity",
            ));
        }
    }
    validate_quality_candidate_snapshots_in(connection, &cohorts).await?;

    let role_records = sqlx::query_as::<_, QualityCandidateRoleRecord>(
        "SELECT id, cohort_id, sequence, role, disposition, predecessor_id, fingerprint, \
         artifact_json, created_at FROM workflow_cohort_role_decisions \
         ORDER BY cohort_id, sequence, id",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut histories = BTreeMap::<Uuid, Vec<(i64, CohortRoleDecision)>>::new();
    for record in role_records {
        let decision: CohortRoleDecision = decode(&record.artifact_json)?;
        if !cohorts.contains_key(&record.cohort_id)
            || record.id != decision.id
            || record.cohort_id != decision.cohort_id
            || record.role != cohort_role(decision.role)
            || record.disposition != cohort_disposition(decision.disposition)
            || record.predecessor_id != decision.predecessor_id
            || record.fingerprint != decision.fingerprint
            || record.created_at != decision.created_at
            || decision
                .reproduce_fingerprint()
                .map_err(|error| adapter_error(error.to_string()))?
                != decision.fingerprint
        {
            return Err(adapter_error(
                "quality candidate cohort role governance failed integrity validation",
            ));
        }
        histories
            .entry(decision.cohort_id)
            .or_default()
            .push((record.sequence, decision));
    }

    for (cohort_id, cohort) in &cohorts {
        let history = histories.get(cohort_id).ok_or_else(|| {
            adapter_error("quality candidate cohort has no append-only role history")
        })?;
        for (index, (sequence, decision)) in history.iter().enumerate() {
            if *sequence
                != i64::try_from(index).map_err(|error| adapter_error(error.to_string()))?
            {
                return Err(adapter_error(
                    "quality candidate cohort role history has a sequence gap",
                ));
            }
            if let Some((_, predecessor)) =
                index.checked_sub(1).and_then(|prior| history.get(prior))
            {
                if decision.predecessor_id != Some(predecessor.id)
                    || decision.predecessor_fingerprint.as_deref()
                        != Some(predecessor.fingerprint.as_str())
                {
                    return Err(adapter_error(
                        "quality candidate cohort role history has a broken predecessor chain",
                    ));
                }
                CohortRoleDecision::transition(
                    predecessor,
                    decision.role,
                    decision.disposition,
                    decision.reason.clone(),
                )
                .map_err(|error| adapter_error(error.to_string()))?;
            } else {
                if decision.predecessor_id.is_some()
                    || decision.predecessor_fingerprint.is_some()
                    || decision.disposition != CohortDisposition::Active
                {
                    return Err(adapter_error(
                        "quality candidate cohort initial role is not an initial active decision",
                    ));
                }
                CohortRoleDecision::initial(cohort, decision.role, decision.reason.clone())
                    .map_err(|error| adapter_error(error.to_string()))?;
            }
        }
    }
    Ok(())
}

async fn validate_quality_candidate_snapshots_in(
    connection: &mut SqliteConnection,
    cohorts: &BTreeMap<Uuid, EvaluationCohort>,
) -> Result<(), QualityAdapterError> {
    if cohorts.is_empty() {
        return Ok(());
    }
    let snapshot_records = sqlx::query_as::<_, SnapshotRecord>(
        "SELECT DISTINCT snapshot.id, snapshot.source_dataset_id, snapshot.name, \
         snapshot.description, snapshot.split_configuration_json, snapshot.member_count, \
         snapshot.fingerprint, snapshot.created_at FROM dataset_snapshots AS snapshot \
         JOIN workflow_evaluation_cohorts AS cohort ON cohort.snapshot_id = snapshot.id \
         ORDER BY snapshot.id",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut snapshots = BTreeMap::new();
    for record in snapshot_records {
        let snapshot = record
            .into_domain()
            .map_err(|error| adapter_error(error.to_string()))?;
        if snapshots.insert(snapshot.id, snapshot).is_some() {
            return Err(adapter_error(
                "quality candidate governance repeats a dataset snapshot identity",
            ));
        }
    }
    if cohorts
        .values()
        .any(|cohort| !snapshots.contains_key(&cohort.snapshot_id))
    {
        return Err(adapter_error(
            "quality candidate cohort dataset snapshot is missing",
        ));
    }

    let member_records = sqlx::query_as::<_, SnapshotMemberRecord>(
        "SELECT member.id, member.snapshot_id, member.source_row_id, member.split, member.text, \
         member.label, member.dimensions_json, member.fields_json, \
         member.source_provenance_json, member.source_created_at \
         FROM dataset_snapshot_members AS member \
         WHERE member.snapshot_id IN (\
             SELECT DISTINCT cohort.snapshot_id FROM workflow_evaluation_cohorts AS cohort\
         ) ORDER BY member.snapshot_id, member.source_row_id",
    )
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut members_by_snapshot = BTreeMap::<Uuid, Vec<SnapshotMember>>::new();
    for record in member_records {
        let member = record
            .into_domain()
            .map_err(|error| adapter_error(error.to_string()))?;
        if !snapshots.contains_key(&member.snapshot_id) {
            return Err(adapter_error(
                "quality candidate cohort member belongs to an unknown snapshot",
            ));
        }
        members_by_snapshot
            .entry(member.snapshot_id)
            .or_default()
            .push(member);
    }
    for (snapshot_id, snapshot) in &snapshots {
        let members = members_by_snapshot
            .get(snapshot_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        validate_snapshot(snapshot, members).map_err(|_| {
            adapter_error(
                "quality candidate cohort snapshot membership failed integrity validation",
            )
        })?;
    }
    Ok(())
}

const fn cohort_origin(value: CohortOrigin) -> &'static str {
    match value {
        CohortOrigin::InternalSnapshot => "internal_snapshot",
        CohortOrigin::ExternalBenchmark => "external_benchmark",
    }
}

const fn cohort_role(value: CohortRole) -> &'static str {
    match value {
        CohortRole::Training => "training",
        CohortRole::Development => "development",
        CohortRole::Diagnostic => "diagnostic",
        CohortRole::SealedAcceptance => "sealed_acceptance",
        CohortRole::ExternalBenchmark => "external_benchmark",
    }
}

const fn cohort_disposition(value: CohortDisposition) -> &'static str {
    match value {
        CohortDisposition::Active => "active",
        CohortDisposition::Retired => "retired",
    }
}

fn decode_source_rows(
    records: Vec<SourceRowRecord>,
) -> Result<Vec<SourceRow>, QualityAdapterError> {
    records
        .into_iter()
        .map(|record| {
            note_source_row_decode();
            record
                .into_domain()
                .map_err(|error| adapter_error(error.to_string()))
        })
        .collect()
}

fn canonical_requested_ids(mut ids: Vec<Uuid>) -> Result<Vec<Uuid>, QualityAdapterError> {
    if ids.iter().any(Uuid::is_nil) {
        return Err(adapter_error("source-row IDs must not be nil"));
    }
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(adapter_error("source-row IDs must be unique"));
    }
    Ok(ids)
}

fn require_exact_source_ids(
    expected: &[Uuid],
    actual: &[SourceRow],
) -> Result<(), QualityAdapterError> {
    let actual = actual.iter().map(|row| row.id).collect::<Vec<_>>();
    if actual == expected {
        Ok(())
    } else {
        Err(adapter_error(
            "one or more requested source rows are missing, ineligible, or belong to another dataset",
        ))
    }
}

async fn load_source_rows_in(
    connection: &mut SqliteConnection,
    dataset_id: Uuid,
    source_ids: &[Uuid],
) -> Result<Vec<SourceRow>, QualityAdapterError> {
    if source_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, dataset_id, text, label, dimensions_json, fields_json, provenance_json, \
         created_at FROM dataset_source_rows WHERE dataset_id = ",
    );
    builder.push_bind(dataset_id).push(" AND id IN (");
    let mut separated = builder.separated(", ");
    for id in source_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(") ORDER BY id");
    let records = builder
        .build_query_as::<SourceRowRecord>()
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    let rows = decode_source_rows(records)?;
    require_exact_source_ids(source_ids, &rows)?;
    Ok(rows)
}

fn validate_plan_run_creation(
    plan: &AuditPlan,
    run: &QualityAuditRun,
    guidance: &EvaluatorGuidance,
) -> Result<(), QualityAdapterError> {
    plan.verify_integrity().map_err(domain_error)?;
    guidance.verify_against(plan).map_err(domain_error)?;
    validate_run(plan, run)?;
    run.verify_against_evidence(plan, &[], &[], &[])
        .map_err(domain_error)?;
    if run.state != QualityAuditRunState::Queued
        || run.started_at.is_some()
        || run.finished_at.is_some()
        || run.stop_reason.is_some()
        || run.error_message.is_some()
    {
        return Err(adapter_error(
            "a new quality audit must begin in its pristine queued state",
        ));
    }
    Ok(())
}

async fn verify_plan_sources_in(
    connection: &mut SqliteConnection,
    plan: &AuditPlan,
) -> Result<(), QualityAdapterError> {
    let ids = plan
        .items
        .iter()
        .map(|item| item.source_row_id)
        .collect::<Vec<_>>();
    let ids = canonical_requested_ids(ids)?;
    let rows =
        load_source_rows_in(connection, plan.dataset_schema.dataset_definition_id, &ids).await?;
    let rows = rows
        .into_iter()
        .map(|row| (row.id, row))
        .collect::<BTreeMap<_, _>>();
    for item in &plan.items {
        let row = &rows[&item.source_row_id];
        let fingerprint =
            artifact_core::fingerprint(row).map_err(|error| adapter_error(error.to_string()))?;
        if fingerprint != item.source_row_fingerprint {
            return Err(adapter_error(format!(
                "source row {} no longer matches the audit plan",
                item.source_row_id
            )));
        }
    }
    Ok(())
}

async fn insert_plan_in(
    connection: &mut SqliteConnection,
    plan: &AuditPlan,
) -> Result<(), QualityAdapterError> {
    sqlx::query(
        "INSERT INTO dataset_quality_audit_plans \
         (id, dataset_id, dataset_fingerprint, policy_fingerprint, source_set_fingerprint, \
          resolved_guidance_fingerprint, evaluator_protocol_version, fingerprint, plan_json, \
          created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(plan.id)
    .bind(plan.dataset_schema.dataset_definition_id)
    .bind(&plan.dataset_schema.dataset_definition_fingerprint)
    .bind(&plan.policy.fingerprint)
    .bind(&plan.source_set_fingerprint)
    .bind(&plan.resolved_guidance_fingerprint)
    .bind(&plan.evaluator_protocol_version)
    .bind(&plan.fingerprint)
    .bind(encode(plan)?)
    .bind(plan.created_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    for item in &plan.items {
        sqlx::query(
            "INSERT INTO dataset_quality_audit_plan_items \
             (plan_id, source_row_id, source_row_fingerprint, cell_json, \
              provenance_stratum_json, selection, item_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(plan.id)
        .bind(item.source_row_id)
        .bind(&item.source_row_fingerprint)
        .bind(encode(&item.cell)?)
        .bind(encode(&item.provenance_stratum)?)
        .bind(audit_selection(item.selection))
        .bind(encode(item)?)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

async fn insert_guidance_in(
    connection: &mut SqliteConnection,
    plan: &AuditPlan,
    guidance: &EvaluatorGuidance,
) -> Result<(), QualityAdapterError> {
    guidance.verify_against(plan).map_err(domain_error)?;
    sqlx::query(
        "INSERT INTO dataset_quality_audit_guidance \
         (plan_id, resolved_guidance_fingerprint, guidance_json) VALUES (?, ?, ?)",
    )
    .bind(plan.id)
    .bind(&plan.resolved_guidance_fingerprint)
    .bind(encode(guidance)?)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

async fn load_guidance_in(
    connection: &mut SqliteConnection,
    plan: &AuditPlan,
) -> Result<Option<EvaluatorGuidance>, QualityAdapterError> {
    let record = sqlx::query(
        "SELECT resolved_guidance_fingerprint, guidance_json \
         FROM dataset_quality_audit_guidance WHERE plan_id = ?",
    )
    .bind(plan.id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let Some(record) = record else {
        return Ok(None);
    };
    let stored_fingerprint: String = record
        .try_get("resolved_guidance_fingerprint")
        .map_err(sql_error)?;
    let guidance_json: String = record.try_get("guidance_json").map_err(sql_error)?;
    let guidance: EvaluatorGuidance = decode(&guidance_json)?;
    guidance.verify_against(plan).map_err(domain_error)?;
    if stored_fingerprint != plan.resolved_guidance_fingerprint {
        return Err(adapter_error(
            "persisted evaluator guidance fingerprint disagrees with the audit plan",
        ));
    }
    Ok(Some(guidance))
}

async fn load_plan_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<AuditPlan>, QualityAdapterError> {
    note_full_plan_load();
    let record = sqlx::query_as::<_, PlanRecord>(
        "SELECT id, dataset_id, dataset_fingerprint, policy_fingerprint, \
         source_set_fingerprint, resolved_guidance_fingerprint, evaluator_protocol_version, \
         fingerprint, plan_json, created_at FROM dataset_quality_audit_plans WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let Some(record) = record else {
        return Ok(None);
    };
    let plan: AuditPlan = decode(&record.plan_json)?;
    plan.verify_integrity().map_err(domain_error)?;
    if record.id != plan.id
        || record.dataset_id != plan.dataset_schema.dataset_definition_id
        || record.dataset_fingerprint != plan.dataset_schema.dataset_definition_fingerprint
        || record.policy_fingerprint != plan.policy.fingerprint
        || record.source_set_fingerprint != plan.source_set_fingerprint
        || record.resolved_guidance_fingerprint != plan.resolved_guidance_fingerprint
        || record.evaluator_protocol_version != plan.evaluator_protocol_version
        || record.fingerprint != plan.fingerprint
        || record.created_at != plan.created_at
    {
        return Err(adapter_error(
            "quality audit plan normalized columns disagree with JSON",
        ));
    }
    let items = sqlx::query_as::<_, PlanItemRecord>(
        "SELECT source_row_id, source_row_fingerprint, cell_json, provenance_stratum_json, \
         selection, item_json FROM dataset_quality_audit_plan_items \
         WHERE plan_id = ? ORDER BY source_row_id",
    )
    .bind(id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let expected = plan
        .items
        .iter()
        .map(|item| (item.source_row_id, item))
        .collect::<BTreeMap<_, _>>();
    if items.len() != expected.len() {
        return Err(adapter_error(
            "quality audit plan item manifest is incomplete",
        ));
    }
    for record in items {
        let item: AuditPlanItem = decode(&record.item_json)?;
        let Some(expected_item) = expected.get(&record.source_row_id) else {
            return Err(adapter_error("quality audit plan item is foreign"));
        };
        if &item != *expected_item
            || record.source_row_fingerprint != item.source_row_fingerprint
            || decode::<dataset_quality_core::population::CellIdentity>(&record.cell_json)?
                != item.cell
            || decode::<dataset_quality_core::population::ProvenanceStratum>(
                &record.provenance_stratum_json,
            )? != item.provenance_stratum
            || record.selection != audit_selection(item.selection)
        {
            return Err(adapter_error(
                "quality audit plan item columns disagree with immutable JSON",
            ));
        }
    }
    Ok(Some(plan))
}

async fn require_persisted_checked_plan_in(
    connection: &mut SqliteConnection,
    checked: &CheckedAuditPlan<'_>,
) -> Result<(), QualityAdapterError> {
    let plan = checked.plan();
    let record = sqlx::query_as::<_, PlanStatusRecord>(
        "SELECT id, dataset_id, dataset_fingerprint, policy_fingerprint, \
         source_set_fingerprint, resolved_guidance_fingerprint, evaluator_protocol_version, \
         fingerprint, created_at FROM dataset_quality_audit_plans WHERE id = ?",
    )
    .bind(plan.id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
    if record.id != plan.id
        || record.dataset_id != plan.dataset_schema.dataset_definition_id
        || record.dataset_fingerprint != plan.dataset_schema.dataset_definition_fingerprint
        || record.policy_fingerprint != plan.policy.fingerprint
        || record.source_set_fingerprint != plan.source_set_fingerprint
        || record.resolved_guidance_fingerprint != plan.resolved_guidance_fingerprint
        || record.evaluator_protocol_version != plan.evaluator_protocol_version
        || record.fingerprint != plan.fingerprint
        || record.created_at != plan.created_at
    {
        return Err(adapter_error(
            "checked audit plan disagrees with persisted immutable identity",
        ));
    }
    Ok(())
}

async fn insert_execution_lease_in(
    connection: &mut SqliteConnection,
    lease: &AuditExecutionLease,
) -> Result<bool, QualityAdapterError> {
    lease.verify().map_err(domain_error)?;
    let result = sqlx::query(
        "INSERT INTO dataset_quality_execution_leases \
         (run_id, invocation_token, process_id, process_started_at, acquired_at) \
         VALUES (?, ?, ?, ?, ?) ON CONFLICT(run_id) DO NOTHING",
    )
    .bind(lease.run_id)
    .bind(lease.invocation_token)
    .bind(i64::from(lease.process_id))
    .bind(as_i64(lease.process_started_at)?)
    .bind(lease.acquired_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    Ok(result.rows_affected() == 1)
}

async fn load_execution_lease_in(
    connection: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Option<AuditExecutionLease>, QualityAdapterError> {
    sqlx::query_as::<_, ExecutionLeaseRecord>(
        "SELECT run_id, invocation_token, process_id, process_started_at, acquired_at \
         FROM dataset_quality_execution_leases WHERE run_id = ?",
    )
    .bind(run_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .map(ExecutionLeaseRecord::into_domain)
    .transpose()
}

async fn replace_execution_lease_in(
    connection: &mut SqliteConnection,
    previous: &AuditExecutionLease,
    next: &AuditExecutionLease,
) -> Result<(), QualityAdapterError> {
    let result = sqlx::query(
        "UPDATE dataset_quality_execution_leases SET invocation_token = ?, process_id = ?, \
         process_started_at = ?, acquired_at = ? WHERE run_id = ? AND invocation_token = ? \
         AND process_id = ? AND process_started_at = ? AND acquired_at = ?",
    )
    .bind(next.invocation_token)
    .bind(i64::from(next.process_id))
    .bind(as_i64(next.process_started_at)?)
    .bind(next.acquired_at)
    .bind(previous.run_id)
    .bind(previous.invocation_token)
    .bind(i64::from(previous.process_id))
    .bind(as_i64(previous.process_started_at)?)
    .bind(previous.acquired_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    require_one(
        result.rows_affected(),
        "stale quality audit execution lease",
    )
}

async fn require_execution_lease_in(
    connection: &mut SqliteConnection,
    expected: &AuditExecutionLease,
) -> Result<(), QualityAdapterError> {
    let actual = load_execution_lease_in(connection, expected.run_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit execution lease is missing"))?;
    if actual != *expected {
        return Err(adapter_error(
            "quality audit execution lease was replaced by another invocation",
        ));
    }
    Ok(())
}

fn execution_lease_owner_is_active(lease: &AuditExecutionLease) -> bool {
    System::new_all()
        .process(Pid::from_u32(lease.process_id))
        .is_some_and(|process| process.start_time() == lease.process_started_at)
}

fn quality_process_started_at(process_id: u32) -> Result<u64, QualityAdapterError> {
    System::new_all()
        .process(Pid::from_u32(process_id))
        .map(sysinfo::Process::start_time)
        .ok_or_else(|| adapter_error("could not inspect the current process for audit leasing"))
}

fn validate_run(plan: &AuditPlan, run: &QualityAuditRun) -> Result<(), QualityAdapterError> {
    run.verify_integrity(plan).map_err(domain_error)?;
    validate_run_timestamps(run)
}

fn validate_run_timestamps(run: &QualityAuditRun) -> Result<(), QualityAdapterError> {
    if run
        .started_at
        .is_some_and(|started| started < run.created_at)
        || run
            .finished_at
            .is_some_and(|finished| finished < run.started_at.unwrap_or(run.created_at))
    {
        return Err(adapter_error("quality audit timestamps are not monotonic"));
    }
    Ok(())
}

fn validate_run_transition(
    previous: &QualityAuditRun,
    next: &QualityAuditRun,
) -> Result<(), QualityAdapterError> {
    let immutable = previous.id == next.id
        && previous.plan_id == next.plan_id
        && previous.plan_fingerprint == next.plan_fingerprint
        && previous.source_set_fingerprint == next.source_set_fingerprint
        && previous.policy_fingerprint == next.policy_fingerprint
        && previous.primary_evaluator == next.primary_evaluator
        && previous.independent_reviewers == next.independent_reviewers
        && previous.created_at == next.created_at
        && previous.specification_fingerprint == next.specification_fingerprint;
    let legal_state = matches!(
        (previous.state, next.state),
        (QualityAuditRunState::Queued, QualityAuditRunState::Running)
            | (
                QualityAuditRunState::Queued,
                QualityAuditRunState::Cancelled
            )
            | (QualityAuditRunState::Running, QualityAuditRunState::Running)
            | (
                QualityAuditRunState::Running,
                QualityAuditRunState::Completed
            )
            | (QualityAuditRunState::Running, QualityAuditRunState::Failed)
            | (
                QualityAuditRunState::Running,
                QualityAuditRunState::Cancelled
            )
    );
    let monotonic_progress = next.progress.population_rows == previous.progress.population_rows
        && next.progress.selected_rows == previous.progress.selected_rows
        && next.progress.invalid_rows >= previous.progress.invalid_rows
        && next
            .progress
            .assessed_rows
            .saturating_add(next.progress.invalid_rows)
            >= previous
                .progress
                .assessed_rows
                .saturating_add(previous.progress.invalid_rows);
    let monotonic_usage = next.usage.evaluator_requests >= previous.usage.evaluator_requests
        && next.usage.request_attempts >= previous.usage.request_attempts
        && next.usage.input_tokens >= previous.usage.input_tokens
        && next.usage.output_tokens >= previous.usage.output_tokens
        && next.usage.total_tokens >= previous.usage.total_tokens
        && next.usage.cost_microusd >= previous.usage.cost_microusd;
    if !immutable
        || !legal_state
        || !monotonic_progress
        || !monotonic_usage
        || (previous.cancel_requested && !next.cancel_requested)
    {
        return Err(adapter_error(
            "quality audit update violates immutable identity or durable state monotonicity",
        ));
    }
    Ok(())
}

async fn insert_run_in(
    connection: &mut SqliteConnection,
    run: &QualityAuditRun,
) -> Result<(), QualityAdapterError> {
    sqlx::query(
        "INSERT INTO dataset_quality_audit_runs \
         (id, plan_id, state, specification_fingerprint, population_rows, selected_rows, \
         assessed_rows, pending_review_rows, qualified_rows, borderline_rows, quarantined_rows, invalid_rows, \
          assessment_count, \
          evaluator_requests, request_attempts, input_tokens, output_tokens, total_tokens, \
          cost_microusd, cancel_requested, run_json, created_at, started_at, finished_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(run.id)
    .bind(run.plan_id)
    .bind(run_state(run.state))
    .bind(&run.specification_fingerprint)
    .bind(as_i64(run.progress.population_rows)?)
    .bind(as_i64(run.progress.selected_rows)?)
    .bind(as_i64(run.progress.assessed_rows)?)
    .bind(as_i64(run.progress.pending_review_rows)?)
    .bind(as_i64(run.progress.qualified_rows)?)
    .bind(as_i64(run.progress.borderline_rows)?)
    .bind(as_i64(run.progress.quarantined_rows)?)
    .bind(as_i64(run.progress.invalid_rows)?)
    .bind(0_i64)
    .bind(i64::from(run.usage.evaluator_requests))
    .bind(i64::from(run.usage.request_attempts))
    .bind(as_i64(run.usage.input_tokens)?)
    .bind(as_i64(run.usage.output_tokens)?)
    .bind(as_i64(run.usage.total_tokens)?)
    .bind(as_i64(run.usage.cost_microusd)?)
    .bind(run.cancel_requested)
    .bind(encode(run)?)
    .bind(run.created_at)
    .bind(run.started_at)
    .bind(run.finished_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

async fn update_run_in(
    connection: &mut SqliteConnection,
    previous: &QualityAuditRun,
    run: &QualityAuditRun,
) -> Result<(), QualityAdapterError> {
    let result = sqlx::query(
        "UPDATE dataset_quality_audit_runs SET state = ?, assessed_rows = ?, pending_review_rows = ?, \
         qualified_rows = ?, borderline_rows = ?, quarantined_rows = ?, invalid_rows = ?, \
         evaluator_requests = ?, request_attempts = ?, input_tokens = ?, output_tokens = ?, \
         total_tokens = ?, cost_microusd = ?, cancel_requested = ?, run_json = ?, \
         started_at = ?, finished_at = ? WHERE id = ? AND run_json = ?",
    )
    .bind(run_state(run.state))
    .bind(as_i64(run.progress.assessed_rows)?)
    .bind(as_i64(run.progress.pending_review_rows)?)
    .bind(as_i64(run.progress.qualified_rows)?)
    .bind(as_i64(run.progress.borderline_rows)?)
    .bind(as_i64(run.progress.quarantined_rows)?)
    .bind(as_i64(run.progress.invalid_rows)?)
    .bind(i64::from(run.usage.evaluator_requests))
    .bind(i64::from(run.usage.request_attempts))
    .bind(as_i64(run.usage.input_tokens)?)
    .bind(as_i64(run.usage.output_tokens)?)
    .bind(as_i64(run.usage.total_tokens)?)
    .bind(as_i64(run.usage.cost_microusd)?)
    .bind(run.cancel_requested)
    .bind(encode(run)?)
    .bind(run.started_at)
    .bind(run.finished_at)
    .bind(run.id)
    .bind(encode(previous)?)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    require_one(result.rows_affected(), "quality audit run")
}

async fn load_run_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<QualityAuditRun>, QualityAdapterError> {
    let Some((run, _)) = load_run_projection_in(connection, id).await? else {
        return Ok(None);
    };
    let plan = load_plan_in(connection, run.plan_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
    validate_run(&plan, &run)?;
    Ok(Some(run))
}

async fn load_run_projection_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<(QualityAuditRun, u64)>, QualityAdapterError> {
    let record = sqlx::query_as::<_, RunRecord>(
        "SELECT id, plan_id, state, specification_fingerprint, population_rows, selected_rows, \
         assessed_rows, pending_review_rows, qualified_rows, borderline_rows, quarantined_rows, invalid_rows, \
         assessment_count, \
         evaluator_requests, request_attempts, input_tokens, output_tokens, total_tokens, \
         cost_microusd, cancel_requested, run_json, created_at, started_at, finished_at \
         FROM dataset_quality_audit_runs WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    record.map(check_run_record).transpose()
}

fn check_run_record(record: RunRecord) -> Result<(QualityAuditRun, u64), QualityAdapterError> {
    let run: QualityAuditRun = decode(&record.run_json)?;
    let progress = AuditProgress {
        population_rows: as_u64(record.population_rows)?,
        selected_rows: as_u64(record.selected_rows)?,
        assessed_rows: as_u64(record.assessed_rows)?,
        pending_review_rows: as_u64(record.pending_review_rows)?,
        qualified_rows: as_u64(record.qualified_rows)?,
        borderline_rows: as_u64(record.borderline_rows)?,
        quarantined_rows: as_u64(record.quarantined_rows)?,
        invalid_rows: as_u64(record.invalid_rows)?,
    };
    let usage = AuditUsage {
        evaluator_requests: as_u32(record.evaluator_requests)?,
        request_attempts: as_u32(record.request_attempts)?,
        input_tokens: as_u64(record.input_tokens)?,
        output_tokens: as_u64(record.output_tokens)?,
        total_tokens: as_u64(record.total_tokens)?,
        cost_microusd: as_u64(record.cost_microusd)?,
    };
    if record.id != run.id
        || record.plan_id != run.plan_id
        || record.state != run_state(run.state)
        || record.specification_fingerprint != run.specification_fingerprint
        || progress != run.progress
        || usage != run.usage
        || record.cancel_requested != run.cancel_requested
        || record.created_at != run.created_at
        || record.started_at != run.started_at
        || record.finished_at != run.finished_at
    {
        return Err(adapter_error(
            "quality audit run normalized columns disagree with JSON",
        ));
    }
    Ok((run, as_u64(record.assessment_count)?))
}

async fn load_audit_status_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<AuditStatusProjection>, QualityAdapterError> {
    let Some((run, assessments)) = load_run_projection_in(connection, id).await? else {
        return Ok(None);
    };
    let plan_record = sqlx::query_as::<_, PlanStatusRecord>(
        "SELECT id, dataset_id, dataset_fingerprint, policy_fingerprint, \
         source_set_fingerprint, resolved_guidance_fingerprint, evaluator_protocol_version, \
         fingerprint, created_at FROM dataset_quality_audit_plans WHERE id = ?",
    )
    .bind(run.plan_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
    let guidance_record = sqlx::query_as::<_, (String, String)>(
        "SELECT resolved_guidance_fingerprint, guidance_json \
         FROM dataset_quality_audit_guidance WHERE plan_id = ?",
    )
    .bind(run.plan_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or_else(|| adapter_error("quality audit guidance is missing"))?;
    let guidance: EvaluatorGuidance = decode(&guidance_record.1)?;
    if guidance.reproduce_fingerprint().map_err(domain_error)?
        != plan_record.resolved_guidance_fingerprint
        || guidance_record.0 != plan_record.resolved_guidance_fingerprint
    {
        return Err(adapter_error(
            "quality audit status guidance does not reproduce from normalized facts",
        ));
    }
    let report_id =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM dataset_quality_reports WHERE run_id = ?")
            .bind(run.id)
            .fetch_optional(&mut *connection)
            .await
            .map_err(sql_error)?;
    let projection = AuditStatusProjection {
        evaluator_requests: u64::from(run.usage.evaluator_requests),
        evaluator_attempts: u64::from(run.usage.request_attempts),
        assessments,
        report_id,
        plan: AuditPlanStatusProjection {
            id: plan_record.id,
            schema_version: AUDIT_PLAN_SCHEMA_VERSION,
            dataset_definition_id: plan_record.dataset_id,
            dataset_definition_fingerprint: plan_record.dataset_fingerprint,
            policy_fingerprint: plan_record.policy_fingerprint,
            resolved_guidance_fingerprint: plan_record.resolved_guidance_fingerprint,
            evaluator_protocol_version: plan_record.evaluator_protocol_version,
            population_rows: run.progress.population_rows,
            selected_rows: run.progress.selected_rows,
            source_set_fingerprint: plan_record.source_set_fingerprint,
            created_at: plan_record.created_at,
            fingerprint: plan_record.fingerprint,
        },
        run,
    };
    projection.verify().map_err(domain_error)?;
    Ok(Some(projection))
}

fn validate_attempt_with_context(
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
    request: &BlindEvaluatorRequest,
    attempt: &EvaluatorAttempt,
) -> Result<(), QualityAdapterError> {
    attempt
        .verify_with_context(checked, run, request)
        .map_err(domain_error)?;
    if attempt.started_at < run.created_at
        || attempt
            .finished_at
            .is_some_and(|finished| finished < attempt.started_at)
    {
        return Err(adapter_error(
            "evaluator attempt timestamps are not monotonic",
        ));
    }
    Ok(())
}

async fn insert_attempt_in(
    connection: &mut SqliteConnection,
    request: &BlindEvaluatorRequest,
    attempt: &EvaluatorAttempt,
) -> Result<(), QualityAdapterError> {
    sqlx::query(
        "INSERT INTO dataset_quality_evaluator_attempts \
         (id, run_id, request_id, request_sequence, attempt_number, request_fingerprint, \
          retry_payload_fingerprint, state, fingerprint, request_json, attempt_json, \
          started_at, finished_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(attempt.id)
    .bind(attempt.run_id)
    .bind(request.id)
    .bind(i64::from(attempt.request_sequence))
    .bind(i64::from(attempt.attempt_number))
    .bind(&attempt.request_fingerprint)
    .bind(&attempt.retry_payload_fingerprint)
    .bind(attempt_state(attempt.state))
    .bind(&attempt.fingerprint)
    .bind(encode(request)?)
    .bind(encode(attempt)?)
    .bind(attempt.started_at)
    .bind(attempt.finished_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    for source_row_id in &attempt.source_row_ids {
        sqlx::query(
            "INSERT INTO dataset_quality_evaluator_attempt_rows (attempt_id, source_row_id) \
             VALUES (?, ?)",
        )
        .bind(attempt.id)
        .bind(source_row_id)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

async fn load_attempt_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<EvaluatorAttempt>, QualityAdapterError> {
    let record = sqlx::query_as::<_, AttemptRecord>(
        "SELECT id, run_id, request_id, request_sequence, attempt_number, request_fingerprint, \
         retry_payload_fingerprint, state, fingerprint, request_json, attempt_json, started_at, \
         finished_at FROM dataset_quality_evaluator_attempts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    match record {
        Some(record) => check_attempt_record(connection, record).await.map(Some),
        None => Ok(None),
    }
}

async fn load_attempt_against_in(
    connection: &mut SqliteConnection,
    id: Uuid,
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
) -> Result<Option<(BlindEvaluatorRequest, EvaluatorAttempt)>, QualityAdapterError> {
    let record = sqlx::query_as::<_, AttemptRecord>(
        "SELECT id, run_id, request_id, request_sequence, attempt_number, request_fingerprint, \
         retry_payload_fingerprint, state, fingerprint, request_json, attempt_json, started_at, \
         finished_at FROM dataset_quality_evaluator_attempts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let Some(record) = record else {
        return Ok(None);
    };
    let source_row_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT source_row_id FROM dataset_quality_evaluator_attempt_rows \
         WHERE attempt_id = ? ORDER BY source_row_id",
    )
    .bind(record.id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    check_attempt_and_request_record_against(record, source_row_ids, checked, run).map(Some)
}

async fn check_attempt_record(
    connection: &mut SqliteConnection,
    record: AttemptRecord,
) -> Result<EvaluatorAttempt, QualityAdapterError> {
    check_attempt_and_request_record(connection, record)
        .await
        .map(|(_, attempt)| attempt)
}

async fn check_attempt_and_request_record(
    connection: &mut SqliteConnection,
    record: AttemptRecord,
) -> Result<(BlindEvaluatorRequest, EvaluatorAttempt), QualityAdapterError> {
    let source_row_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT source_row_id FROM dataset_quality_evaluator_attempt_rows \
         WHERE attempt_id = ? ORDER BY source_row_id",
    )
    .bind(record.id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let run = load_run_in(connection, record.run_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit run is missing"))?;
    let plan = load_plan_in(connection, run.plan_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
    let checked = CheckedAuditPlan::new(&plan).map_err(domain_error)?;
    check_attempt_and_request_record_against(record, source_row_ids, &checked, &run)
}

fn check_attempt_and_request_record_against(
    record: AttemptRecord,
    source_row_ids: Vec<Uuid>,
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
) -> Result<(BlindEvaluatorRequest, EvaluatorAttempt), QualityAdapterError> {
    let request: BlindEvaluatorRequest = decode(&record.request_json)?;
    let attempt: EvaluatorAttempt = decode(&record.attempt_json)?;
    if record.id != attempt.id
        || record.run_id != attempt.run_id
        || record.run_id != run.id
        || record.request_id != request.id
        || record.request_id != attempt.request_id
        || as_u32(record.request_sequence)? != attempt.request_sequence
        || as_u32(record.request_sequence)? != request.request_sequence
        || as_u32(record.attempt_number)? != attempt.attempt_number
        || as_u32(record.attempt_number)? != request.attempt_number
        || record.request_fingerprint != attempt.request_fingerprint
        || record.request_fingerprint != request.fingerprint
        || record.retry_payload_fingerprint != attempt.retry_payload_fingerprint
        || record.state != attempt_state(attempt.state)
        || record.fingerprint != attempt.fingerprint
        || record.started_at != attempt.started_at
        || record.finished_at != attempt.finished_at
        || source_row_ids != attempt.source_row_ids
        || request.audit_run_id != record.run_id
        || request.attempt_id != record.id
    {
        return Err(adapter_error(
            "evaluator request or attempt normalized facts disagree with JSON",
        ));
    }
    let evaluator = run
        .evaluator_for_fingerprint(&request.evaluator_identity_fingerprint)
        .ok_or_else(|| adapter_error("evaluator request uses an evaluator outside its run"))?;
    request
        .verify_with_context(checked, evaluator)
        .map_err(domain_error)?;
    validate_attempt_with_context(checked, run, &request, &attempt)?;
    Ok((request, attempt))
}

async fn load_request_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<BlindEvaluatorRequest>, QualityAdapterError> {
    let record = sqlx::query_as::<_, AttemptRecord>(
        "SELECT id, run_id, request_id, request_sequence, attempt_number, request_fingerprint, \
         retry_payload_fingerprint, state, fingerprint, request_json, attempt_json, started_at, \
         finished_at FROM dataset_quality_evaluator_attempts WHERE request_id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    match record {
        Some(record) => check_attempt_and_request_record(connection, record)
            .await
            .map(|(request, _)| Some(request)),
        None => Ok(None),
    }
}

async fn load_attempts_for_run_in(
    connection: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Vec<EvaluatorAttempt>, QualityAdapterError> {
    let run = load_run_in(connection, run_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit run is missing"))?;
    let plan = load_plan_in(connection, run.plan_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
    load_checked_attempt_evidence_for_run_in(connection, &plan, &run)
        .await
        .map(|(_, attempts)| attempts)
}

async fn load_requests_for_run_in(
    connection: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Vec<BlindEvaluatorRequest>, QualityAdapterError> {
    let run = load_run_in(connection, run_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit run is missing"))?;
    let plan = load_plan_in(connection, run.plan_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
    load_checked_attempt_evidence_for_run_in(connection, &plan, &run)
        .await
        .map(|(requests, _)| requests)
}

async fn load_checked_attempt_evidence_for_run_in(
    connection: &mut SqliteConnection,
    plan: &AuditPlan,
    run: &QualityAuditRun,
) -> Result<(Vec<BlindEvaluatorRequest>, Vec<EvaluatorAttempt>), QualityAdapterError> {
    note_full_evidence_load();
    let checked = CheckedAuditPlan::new(plan).map_err(domain_error)?;
    let records = load_attempt_records_for_run_in(connection, run.id).await?;
    let memberships = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT rows.attempt_id, rows.source_row_id \
         FROM dataset_quality_evaluator_attempt_rows AS rows \
         JOIN dataset_quality_evaluator_attempts AS attempts ON attempts.id = rows.attempt_id \
         WHERE attempts.run_id = ? ORDER BY rows.attempt_id, rows.source_row_id",
    )
    .bind(run.id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut source_ids_by_attempt = BTreeMap::<Uuid, Vec<Uuid>>::new();
    for (attempt_id, source_row_id) in memberships {
        source_ids_by_attempt
            .entry(attempt_id)
            .or_default()
            .push(source_row_id);
    }
    let mut requests = Vec::with_capacity(records.len());
    let mut attempts = Vec::with_capacity(records.len());
    for record in records {
        let source_row_ids = source_ids_by_attempt.remove(&record.id).unwrap_or_default();
        let (request, attempt) =
            check_attempt_and_request_record_against(record, source_row_ids, &checked, run)?;
        requests.push(request);
        attempts.push(attempt);
    }
    if !source_ids_by_attempt.is_empty() {
        return Err(adapter_error(
            "evaluator attempt membership contains an unknown run attempt",
        ));
    }
    Ok((requests, attempts))
}

async fn load_checked_attempt_evidence_for_rows_with_context_in(
    connection: &mut SqliteConnection,
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
    source_row_ids: &[Uuid],
) -> Result<(Vec<BlindEvaluatorRequest>, Vec<EvaluatorAttempt>), QualityAdapterError> {
    let requested = source_row_ids.iter().copied().collect::<BTreeSet<_>>();
    if requested.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut records_builder = QueryBuilder::<Sqlite>::new(
        "SELECT DISTINCT attempts.id, attempts.run_id, attempts.request_id, \
         attempts.request_sequence, attempts.attempt_number, attempts.request_fingerprint, \
         attempts.retry_payload_fingerprint, attempts.state, attempts.fingerprint, \
         attempts.request_json, attempts.attempt_json, attempts.started_at, attempts.finished_at \
         FROM dataset_quality_evaluator_attempts AS attempts \
         JOIN dataset_quality_evaluator_attempt_rows AS rows ON rows.attempt_id = attempts.id \
         WHERE attempts.run_id = ",
    );
    records_builder
        .push_bind(run.id)
        .push(" AND rows.source_row_id IN (");
    let mut separated = records_builder.separated(", ");
    for source_row_id in &requested {
        separated.push_bind(source_row_id);
    }
    separated.push_unseparated(
        ") ORDER BY attempts.request_sequence, attempts.attempt_number, attempts.id",
    );
    let records = records_builder
        .build_query_as::<AttemptRecord>()
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    if records.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let attempt_ids = records.iter().map(|record| record.id).collect::<Vec<_>>();
    let mut memberships_builder = QueryBuilder::<Sqlite>::new(
        "SELECT attempt_id, source_row_id FROM dataset_quality_evaluator_attempt_rows \
         WHERE attempt_id IN (",
    );
    let mut separated = memberships_builder.separated(", ");
    for attempt_id in &attempt_ids {
        separated.push_bind(attempt_id);
    }
    separated.push_unseparated(") ORDER BY attempt_id, source_row_id");
    let memberships = memberships_builder
        .build_query_as::<(Uuid, Uuid)>()
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    let mut source_ids_by_attempt = BTreeMap::<Uuid, Vec<Uuid>>::new();
    for (attempt_id, source_row_id) in memberships {
        source_ids_by_attempt
            .entry(attempt_id)
            .or_default()
            .push(source_row_id);
    }

    let mut requests = Vec::with_capacity(records.len());
    let mut attempts = Vec::with_capacity(records.len());
    for record in records {
        let source_row_ids = source_ids_by_attempt.remove(&record.id).unwrap_or_default();
        let (request, attempt) =
            check_attempt_and_request_record_against(record, source_row_ids, checked, run)?;
        requests.push(request);
        attempts.push(attempt);
    }
    if !source_ids_by_attempt.is_empty() {
        return Err(adapter_error(
            "row-local evaluator attempt membership contains an unknown attempt",
        ));
    }
    Ok((requests, attempts))
}

async fn validate_attempt_reservation_with_context_in(
    connection: &mut SqliteConnection,
    checked: &CheckedAuditPlan<'_>,
    previous_run: &QualityAuditRun,
    request: &BlindEvaluatorRequest,
    attempt: &EvaluatorAttempt,
) -> Result<(), QualityAdapterError> {
    let plan = checked.plan();
    let (_, touching_attempts) = load_checked_attempt_evidence_for_rows_with_context_in(
        connection,
        checked,
        previous_run,
        &attempt.source_row_ids,
    )
    .await?;
    if touching_attempts.iter().any(|previous| {
        previous.request_sequence != attempt.request_sequence
            && previous.evaluator.fingerprint == attempt.evaluator.fingerprint
            && previous
                .source_row_ids
                .iter()
                .any(|source_row_id| attempt.source_row_ids.contains(source_row_id))
    }) {
        return Err(adapter_error(
            "an evaluator may own a source row in only one logical request",
        ));
    }

    let sequence = touching_attempts
        .iter()
        .filter(|previous| previous.request_sequence == attempt.request_sequence)
        .collect::<Vec<_>>();
    if attempt.attempt_number == 1 {
        let expected_sequence = previous_run
            .usage
            .evaluator_requests
            .checked_add(1)
            .ok_or_else(|| adapter_error("evaluator request sequence overflowed"))?;
        if !sequence.is_empty() || attempt.request_sequence != expected_sequence {
            return Err(adapter_error(
                "new logical evaluator request sequence is not canonical",
            ));
        }
        return Ok(());
    }

    if sequence.len() + 1 != attempt.attempt_number as usize
        || attempt.attempt_number > plan.policy.budgets.maximum_attempts_per_request
    {
        return Err(adapter_error(
            "evaluator retry attempt number is not consecutive or exceeds its budget",
        ));
    }
    for (index, previous) in sequence.iter().enumerate() {
        if previous.attempt_number as usize != index + 1
            || !matches!(
                previous.state,
                EvaluatorAttemptState::Failed | EvaluatorAttemptState::Interrupted
            )
            || previous.retry_payload_fingerprint != attempt.retry_payload_fingerprint
            || previous.evaluator != attempt.evaluator
            || previous.source_row_ids != attempt.source_row_ids
            || request
                .reproduce_retry_payload_fingerprint()
                .map_err(domain_error)?
                != previous.retry_payload_fingerprint
        {
            return Err(adapter_error(
                "evaluator retry does not continue its exact durable logical request",
            ));
        }
    }
    Ok(())
}

async fn load_attempt_records_for_run_in(
    connection: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Vec<AttemptRecord>, QualityAdapterError> {
    sqlx::query_as::<_, AttemptRecord>(
        "SELECT id, run_id, request_id, request_sequence, attempt_number, request_fingerprint, \
         retry_payload_fingerprint, state, fingerprint, request_json, attempt_json, started_at, \
         finished_at FROM dataset_quality_evaluator_attempts WHERE run_id = ? \
         ORDER BY request_sequence, attempt_number, id",
    )
    .bind(run_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)
}

fn validate_attempt_finish_with_context(
    checked: &CheckedAuditPlan<'_>,
    previous_run: &QualityAuditRun,
    request: &BlindEvaluatorRequest,
    previous: &EvaluatorAttempt,
    next: &EvaluatorAttempt,
) -> Result<(), QualityAdapterError> {
    validate_attempt_with_context(checked, previous_run, request, previous)?;
    validate_attempt_with_context(checked, previous_run, request, next)?;
    if previous.state != EvaluatorAttemptState::Started
        || next.state == EvaluatorAttemptState::Started
        || previous.id != next.id
        || previous.run_id != next.run_id
        || previous.request_id != next.request_id
        || previous.request_sequence != next.request_sequence
        || previous.attempt_number != next.attempt_number
        || previous.evaluator != next.evaluator
        || previous.source_row_ids != next.source_row_ids
        || previous.request_fingerprint != next.request_fingerprint
        || previous.retry_payload_fingerprint != next.retry_payload_fingerprint
        || previous.started_at != next.started_at
    {
        return Err(adapter_error(
            "only the exact persisted started evaluator attempt may finish",
        ));
    }
    Ok(())
}

async fn update_attempt_in(
    connection: &mut SqliteConnection,
    previous: &EvaluatorAttempt,
    attempt: &EvaluatorAttempt,
) -> Result<(), QualityAdapterError> {
    let result = sqlx::query(
        "UPDATE dataset_quality_evaluator_attempts SET state = ?, fingerprint = ?, \
         attempt_json = ?, finished_at = ? WHERE id = ? AND state = 'started' \
         AND fingerprint = ? AND attempt_json = ?",
    )
    .bind(attempt_state(attempt.state))
    .bind(&attempt.fingerprint)
    .bind(encode(attempt)?)
    .bind(attempt.finished_at)
    .bind(attempt.id)
    .bind(&previous.fingerprint)
    .bind(encode(previous)?)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    require_one(result.rows_affected(), "started evaluator attempt")
}

fn validate_finished_assessments_with_context(
    checked: &CheckedAuditPlan<'_>,
    request: &BlindEvaluatorRequest,
    attempt: &EvaluatorAttempt,
    assessments: &[RowQualityAssessment],
) -> Result<(), QualityAdapterError> {
    if attempt.state != EvaluatorAttemptState::Succeeded && !assessments.is_empty() {
        return Err(adapter_error(
            "only a succeeded evaluator attempt may persist assessments",
        ));
    }
    if attempt.state == EvaluatorAttemptState::Succeeded
        && assessments.len() != attempt.source_row_ids.len()
    {
        return Err(adapter_error(
            "a succeeded evaluator attempt must assess every requested source row",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut rows = BTreeSet::new();
    for assessment in assessments {
        assessment
            .verify_request_binding_with_context(checked, request)
            .map_err(domain_error)?;
        if !ids.insert(assessment.id)
            || !rows.insert(assessment.source_row_id)
            || assessment.audit_run_id != attempt.run_id
            || assessment.attempt_id != attempt.id
            || assessment.request_id != attempt.request_id
            || assessment.request_sequence != attempt.request_sequence
            || assessment.attempt_number != attempt.attempt_number
            || assessment.request_fingerprint != attempt.request_fingerprint
            || !attempt.source_row_ids.contains(&assessment.source_row_id)
        {
            return Err(adapter_error(
                "assessment is duplicated or does not belong to the finished evaluator attempt",
            ));
        }
    }
    if attempt.state == EvaluatorAttemptState::Succeeded
        && rows != attempt.source_row_ids.iter().copied().collect()
    {
        return Err(adapter_error(
            "a succeeded evaluator attempt assessment set differs from its requested rows",
        ));
    }
    Ok(())
}

async fn insert_assessment_in(
    connection: &mut SqliteConnection,
    run_id: Uuid,
    assessment: &RowQualityAssessment,
) -> Result<(), QualityAdapterError> {
    sqlx::query(
        "INSERT INTO dataset_quality_row_assessments \
         (id, run_id, plan_id, attempt_id, request_id, source_row_id, verdict, fingerprint, \
          assessment_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(assessment.id)
    .bind(run_id)
    .bind(assessment.audit_plan_id)
    .bind(assessment.attempt_id)
    .bind(assessment.request_id)
    .bind(assessment.source_row_id)
    .bind(verdict(assessment.verdict))
    .bind(&assessment.fingerprint)
    .bind(encode(assessment)?)
    .bind(assessment.created_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    Ok(())
}

async fn increment_assessment_count_in(
    connection: &mut SqliteConnection,
    run_id: Uuid,
    added: usize,
) -> Result<(), QualityAdapterError> {
    let added = i64::try_from(added).map_err(|error| adapter_error(error.to_string()))?;
    let result = sqlx::query(
        "UPDATE dataset_quality_audit_runs SET assessment_count = assessment_count + ? \
         WHERE id = ?",
    )
    .bind(added)
    .bind(run_id)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    require_one(result.rows_affected(), "quality audit assessment count")
}

async fn require_assessment_count(
    connection: &mut SqliteConnection,
    run_id: Uuid,
    actual: usize,
) -> Result<(), QualityAdapterError> {
    let stored: i64 =
        sqlx::query_scalar("SELECT assessment_count FROM dataset_quality_audit_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&mut *connection)
            .await
            .map_err(sql_error)?;
    if as_u64(stored)? != u64::try_from(actual).map_err(|error| adapter_error(error.to_string()))? {
        return Err(adapter_error(
            "quality audit assessment count disagrees with durable evidence",
        ));
    }
    Ok(())
}

async fn load_assessments_for_attempt_against_in(
    connection: &mut SqliteConnection,
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
    attempt: &EvaluatorAttempt,
    request: &BlindEvaluatorRequest,
) -> Result<Vec<RowQualityAssessment>, QualityAdapterError> {
    let records = sqlx::query_as::<_, AssessmentRecord>(
        "SELECT id, run_id, plan_id, attempt_id, request_id, source_row_id, verdict, \
         fingerprint, assessment_json, created_at FROM dataset_quality_row_assessments \
         WHERE attempt_id = ? ORDER BY source_row_id, created_at, id",
    )
    .bind(attempt.id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    records
        .into_iter()
        .map(|record| check_assessment_record_against(record, checked, run, attempt, request))
        .collect()
}

async fn load_checked_row_local_result_evidence_with_context_in(
    connection: &mut SqliteConnection,
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
    source_row_ids: &[Uuid],
) -> Result<(Vec<RowQualityAssessment>, Vec<EvaluatorAttempt>), QualityAdapterError> {
    let requested = source_row_ids.iter().copied().collect::<BTreeSet<_>>();
    if requested.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let (requests, attempts) = load_checked_attempt_evidence_for_rows_with_context_in(
        connection,
        checked,
        run,
        source_row_ids,
    )
    .await?;
    let requests_by_id = requests
        .iter()
        .map(|request| (request.id, request))
        .collect::<BTreeMap<_, _>>();
    let attempts_by_id = attempts
        .iter()
        .map(|attempt| (attempt.id, attempt))
        .collect::<BTreeMap<_, _>>();

    let mut records_builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, run_id, plan_id, attempt_id, request_id, source_row_id, verdict, \
         fingerprint, assessment_json, created_at FROM dataset_quality_row_assessments \
         WHERE run_id = ",
    );
    records_builder
        .push_bind(run.id)
        .push(" AND source_row_id IN (");
    let mut separated = records_builder.separated(", ");
    for source_row_id in &requested {
        separated.push_bind(source_row_id);
    }
    separated.push_unseparated(") ORDER BY source_row_id, created_at, id");
    let records = records_builder
        .build_query_as::<AssessmentRecord>()
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    let mut assessments = Vec::with_capacity(records.len());
    for record in records {
        let attempt = attempts_by_id
            .get(&record.attempt_id)
            .ok_or_else(|| adapter_error("row-local assessment attempt is missing"))?;
        let request = requests_by_id
            .get(&record.request_id)
            .ok_or_else(|| adapter_error("row-local assessment request is missing"))?;
        assessments.push(check_assessment_record_against(
            record, checked, run, attempt, request,
        )?);
    }
    let invalid_attempts = attempts
        .into_iter()
        .filter(|attempt| attempt.state == EvaluatorAttemptState::InvalidResponse)
        .collect();
    Ok((assessments, invalid_attempts))
}

async fn load_assessment_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<RowQualityAssessment>, QualityAdapterError> {
    let record = sqlx::query_as::<_, AssessmentRecord>(
        "SELECT id, run_id, plan_id, attempt_id, request_id, source_row_id, verdict, \
         fingerprint, assessment_json, created_at FROM dataset_quality_row_assessments \
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    match record {
        Some(record) => check_assessment_record(connection, record).await.map(Some),
        None => Ok(None),
    }
}

async fn load_assessments_for_run_in(
    connection: &mut SqliteConnection,
    run_id: Uuid,
) -> Result<Vec<RowQualityAssessment>, QualityAdapterError> {
    let run = load_run_in(connection, run_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit run is missing"))?;
    let plan = load_plan_in(connection, run.plan_id)
        .await?
        .ok_or_else(|| adapter_error("quality audit plan is missing"))?;
    let (requests, attempts) =
        load_checked_attempt_evidence_for_run_in(connection, &plan, &run).await?;
    let checked = CheckedAuditPlan::new(&plan).map_err(domain_error)?;
    let requests_by_id = requests
        .iter()
        .map(|request| (request.id, request))
        .collect::<BTreeMap<_, _>>();
    let attempts_by_id = attempts
        .iter()
        .map(|attempt| (attempt.id, attempt))
        .collect::<BTreeMap<_, _>>();
    let records = sqlx::query_as::<_, AssessmentRecord>(
        "SELECT id, run_id, plan_id, attempt_id, request_id, source_row_id, verdict, \
         fingerprint, assessment_json, created_at FROM dataset_quality_row_assessments \
         WHERE run_id = ? ORDER BY source_row_id, created_at, id",
    )
    .bind(run_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut values = Vec::with_capacity(records.len());
    for record in records {
        let attempt = attempts_by_id
            .get(&record.attempt_id)
            .ok_or_else(|| adapter_error("assessment evaluator attempt is missing"))?;
        let request = requests_by_id
            .get(&record.request_id)
            .ok_or_else(|| adapter_error("assessment evaluator request is missing"))?;
        values.push(check_assessment_record_against(
            record, &checked, &run, attempt, request,
        )?);
    }
    Ok(values)
}

async fn check_assessment_record(
    connection: &mut SqliteConnection,
    record: AssessmentRecord,
) -> Result<RowQualityAssessment, QualityAdapterError> {
    note_single_assessment_hydration();
    let attempt = load_attempt_in(connection, record.attempt_id)
        .await?
        .ok_or_else(|| adapter_error("assessment evaluator attempt is missing"))?;
    let request = load_request_in(connection, record.request_id)
        .await?
        .ok_or_else(|| adapter_error("assessment evaluator request is missing"))?;
    let run = load_run_in(connection, record.run_id)
        .await?
        .ok_or_else(|| adapter_error("assessment audit run is missing"))?;
    let plan = load_plan_in(connection, record.plan_id)
        .await?
        .ok_or_else(|| adapter_error("assessment audit plan is missing"))?;
    let checked = CheckedAuditPlan::new(&plan).map_err(domain_error)?;
    check_assessment_record_against(record, &checked, &run, &attempt, &request)
}

fn check_assessment_record_against(
    record: AssessmentRecord,
    checked: &CheckedAuditPlan<'_>,
    run: &QualityAuditRun,
    attempt: &EvaluatorAttempt,
    request: &BlindEvaluatorRequest,
) -> Result<RowQualityAssessment, QualityAdapterError> {
    let plan = checked.plan();
    let assessment: RowQualityAssessment = decode(&record.assessment_json)?;
    assessment
        .verify_request_binding_with_context(checked, request)
        .map_err(domain_error)?;
    if record.id != assessment.id
        || record.run_id != attempt.run_id
        || record.run_id != assessment.audit_run_id
        || record.run_id != run.id
        || record.plan_id != plan.id
        || record.plan_id != assessment.audit_plan_id
        || record.attempt_id != assessment.attempt_id
        || record.request_id != assessment.request_id
        || record.request_id != request.id
        || record.source_row_id != assessment.source_row_id
        || record.verdict != verdict(assessment.verdict)
        || record.fingerprint != assessment.fingerprint
        || record.created_at != assessment.created_at
        || assessment.request_sequence != attempt.request_sequence
        || assessment.attempt_number != attempt.attempt_number
        || assessment.request_fingerprint != attempt.request_fingerprint
        || !attempt.source_row_ids.contains(&assessment.source_row_id)
    {
        return Err(adapter_error(
            "assessment normalized facts disagree with JSON or attempt",
        ));
    }
    Ok(assessment)
}

fn canonical_assessments(mut values: Vec<RowQualityAssessment>) -> Vec<RowQualityAssessment> {
    values.sort_by_key(|value| value.id);
    values
}

async fn validate_report_in(
    connection: &mut SqliteConnection,
    report: &DatasetQualityReport,
) -> Result<(), QualityAdapterError> {
    report.verify_integrity().map_err(domain_error)?;
    let plan = load_plan_in(connection, report.plan_id)
        .await?
        .ok_or_else(|| adapter_error("quality report audit plan is missing"))?;
    let run = load_run_in(connection, report.run_id)
        .await?
        .ok_or_else(|| adapter_error("quality report audit run is missing"))?;
    let assessments = load_assessments_for_run_in(connection, run.id).await?;
    let requests = load_requests_for_run_in(connection, run.id).await?;
    let attempts = load_attempts_for_run_in(connection, run.id).await?;
    report
        .verify_against(&plan, &run, &requests, &attempts, &assessments)
        .map_err(domain_error)
}

async fn insert_report_in(
    connection: &mut SqliteConnection,
    report: &DatasetQualityReport,
) -> Result<(), QualityAdapterError> {
    sqlx::query(
        "INSERT INTO dataset_quality_reports \
         (id, run_id, plan_id, dataset_id, assessment_set_fingerprint, \
          invalid_attempt_set_fingerprint, fingerprint, report_json, created_at) \
          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(report.id)
    .bind(report.run_id)
    .bind(report.plan_id)
    .bind(report.dataset_definition_id)
    .bind(&report.assessment_set_fingerprint)
    .bind(&report.invalid_attempt_set_fingerprint)
    .bind(&report.fingerprint)
    .bind(encode(report)?)
    .bind(report.created_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    for row in &report.rows {
        sqlx::query(
            "INSERT INTO dataset_quality_report_rows \
             (report_id, source_row_id, verdict, assessment_set_fingerprint, \
              invalid_attempt_set_fingerprint, fingerprint, row_json) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(report.id)
        .bind(row.source_row_id)
        .bind(report_row_verdict(row.verdict))
        .bind(&row.assessment_set_fingerprint)
        .bind(&row.invalid_attempt_set_fingerprint)
        .bind(&row.fingerprint)
        .bind(encode(row)?)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

async fn load_report_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<DatasetQualityReport>, QualityAdapterError> {
    let record = sqlx::query_as::<_, ReportRecord>(
        "SELECT id, run_id, plan_id, dataset_id, assessment_set_fingerprint, \
         invalid_attempt_set_fingerprint, fingerprint, report_json, created_at \
         FROM dataset_quality_reports WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let Some(record) = record else {
        return Ok(None);
    };
    let report: DatasetQualityReport = decode(&record.report_json)?;
    if record.id != report.id
        || record.run_id != report.run_id
        || record.plan_id != report.plan_id
        || record.dataset_id != report.dataset_definition_id
        || record.assessment_set_fingerprint != report.assessment_set_fingerprint
        || record.invalid_attempt_set_fingerprint != report.invalid_attempt_set_fingerprint
        || record.fingerprint != report.fingerprint
        || record.created_at != report.created_at
    {
        return Err(adapter_error(
            "dataset quality report normalized columns disagree with JSON",
        ));
    }
    let rows = sqlx::query(
        "SELECT source_row_id, verdict, assessment_set_fingerprint, \
         invalid_attempt_set_fingerprint, fingerprint, row_json FROM dataset_quality_report_rows \
         WHERE report_id = ? ORDER BY source_row_id",
    )
    .bind(id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    if rows.len() != report.rows.len() {
        return Err(adapter_error(
            "dataset quality report row manifest is incomplete",
        ));
    }
    for (record, expected) in rows.into_iter().zip(&report.rows) {
        let source_row_id: Uuid = record.try_get("source_row_id").map_err(sql_error)?;
        let stored_verdict: String = record.try_get("verdict").map_err(sql_error)?;
        let assessment_set_fingerprint: String = record
            .try_get("assessment_set_fingerprint")
            .map_err(sql_error)?;
        let invalid_attempt_set_fingerprint: String = record
            .try_get("invalid_attempt_set_fingerprint")
            .map_err(sql_error)?;
        let fingerprint: String = record.try_get("fingerprint").map_err(sql_error)?;
        let row_json: String = record.try_get("row_json").map_err(sql_error)?;
        let value: DatasetQualityReportRow = decode(&row_json)?;
        if &value != expected
            || source_row_id != expected.source_row_id
            || stored_verdict != report_row_verdict(expected.verdict)
            || assessment_set_fingerprint != expected.assessment_set_fingerprint
            || invalid_attempt_set_fingerprint != expected.invalid_attempt_set_fingerprint
            || fingerprint != expected.fingerprint
        {
            return Err(adapter_error(
                "dataset quality report row normalized facts disagree with JSON",
            ));
        }
    }
    validate_report_in(connection, &report).await?;
    Ok(Some(report))
}

fn validate_row_review(
    report: &DatasetQualityReport,
    review: &RowQualityReview,
) -> Result<(), QualityAdapterError> {
    let row = report
        .row(review.source_row_id)
        .ok_or_else(|| adapter_error("row review targets a row outside its report"))?;
    if review.id.is_nil()
        || review.report_id != report.id
        || review.report_fingerprint != report.fingerprint
        || review.report_row_fingerprint != row.fingerprint
        || review.reviewer.trim().is_empty()
        || review.reason.trim().is_empty()
        || review.predecessor_id.is_some() != review.predecessor_fingerprint.is_some()
        || review.reproduce_fingerprint().map_err(domain_error)? != review.fingerprint
    {
        return Err(adapter_error("row review immutable evidence is invalid"));
    }
    Ok(())
}

async fn load_row_review_by_id_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<RowQualityReview>, QualityAdapterError> {
    let record = sqlx::query_as::<_, RowReviewRecord>(
        "SELECT id, report_id, source_row_id, predecessor_id, decision, fingerprint, \
         review_json, created_at FROM dataset_quality_row_reviews WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    record.map(check_row_review_record).transpose()
}

async fn load_referenced_row_reviews_in(
    connection: &mut SqliteConnection,
    references: &[ArtifactReference],
) -> Result<Vec<RowQualityReview>, QualityAdapterError> {
    if references.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, report_id, source_row_id, predecessor_id, decision, fingerprint, \
         review_json, created_at FROM dataset_quality_row_reviews WHERE id IN (",
    );
    let mut separated = builder.separated(", ");
    for reference in references {
        separated.push_bind(reference.id);
    }
    separated.push_unseparated(")");
    let records = builder
        .build_query_as::<RowReviewRecord>()
        .fetch_all(&mut *connection)
        .await
        .map_err(sql_error)?;
    let mut by_id = BTreeMap::new();
    for record in records {
        let review = check_row_review_record(record)?;
        if by_id.insert(review.id, review).is_some() {
            return Err(adapter_error("curation row review identity is duplicated"));
        }
    }
    let mut reviews = Vec::with_capacity(references.len());
    for reference in references {
        let review = by_id
            .remove(&reference.id)
            .ok_or_else(|| adapter_error("curation proposal row review is missing"))?;
        if review.fingerprint != reference.fingerprint {
            return Err(adapter_error(
                "curation proposal row review fingerprint mismatch",
            ));
        }
        reviews.push(review);
    }
    if !by_id.is_empty() {
        return Err(adapter_error(
            "curation proposal row review query returned an unexpected identity",
        ));
    }
    Ok(reviews)
}

fn check_row_review_record(
    record: RowReviewRecord,
) -> Result<RowQualityReview, QualityAdapterError> {
    let review: RowQualityReview = decode(&record.review_json)?;
    if record.id != review.id
        || record.report_id != review.report_id
        || record.source_row_id != review.source_row_id
        || record.predecessor_id != review.predecessor_id
        || record.decision != row_review_decision(review.decision)
        || record.fingerprint != review.fingerprint
        || record.created_at != review.created_at
    {
        return Err(adapter_error(
            "row review normalized columns disagree with JSON",
        ));
    }
    Ok(review)
}

async fn latest_row_review_in(
    connection: &mut SqliteConnection,
    report_id: Uuid,
    source_row_id: Uuid,
) -> Result<Option<RowQualityReview>, QualityAdapterError> {
    let record = sqlx::query_as::<_, RowReviewRecord>(
        "SELECT id, report_id, source_row_id, predecessor_id, decision, fingerprint, \
         review_json, created_at FROM dataset_quality_row_reviews \
         WHERE report_id = ? AND source_row_id = ? ORDER BY rowid DESC LIMIT 1",
    )
    .bind(report_id)
    .bind(source_row_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    record.map(check_row_review_record).transpose()
}

async fn load_row_reviews_in(
    connection: &mut SqliteConnection,
    report_id: Uuid,
) -> Result<Vec<RowQualityReview>, QualityAdapterError> {
    let report = load_report_in(connection, report_id)
        .await?
        .ok_or_else(|| adapter_error("dataset quality report is missing"))?;
    let records = sqlx::query_as::<_, RowReviewRecord>(
        "SELECT id, report_id, source_row_id, predecessor_id, decision, fingerprint, \
         review_json, created_at FROM dataset_quality_row_reviews \
         WHERE report_id = ? ORDER BY source_row_id, rowid",
    )
    .bind(report_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut values = Vec::with_capacity(records.len());
    let mut latest = BTreeMap::<Uuid, (Uuid, String)>::new();
    for record in records {
        let review = check_row_review_record(record)?;
        validate_row_review(&report, &review)?;
        require_review_predecessor(
            review.predecessor_id,
            review.predecessor_fingerprint.as_deref(),
            latest
                .get(&review.source_row_id)
                .map(|(id, fingerprint)| (*id, fingerprint.as_str())),
            "row review",
        )?;
        latest.insert(
            review.source_row_id,
            (review.id, review.fingerprint.clone()),
        );
        values.push(review);
    }
    Ok(values)
}

async fn validate_proposal_in(
    connection: &mut SqliteConnection,
    proposal: &CurationProposal,
) -> Result<(), QualityAdapterError> {
    let report = load_report_in(connection, proposal.report_id)
        .await?
        .ok_or_else(|| adapter_error("curation proposal report is missing"))?;
    let predecessor = match proposal.predecessor_id {
        Some(id) => Some(
            load_proposal_shallow_in(connection, id)
                .await?
                .ok_or_else(|| adapter_error("curation proposal predecessor is missing"))?,
        ),
        None => None,
    };
    if predecessor.as_ref().map(|value| value.fingerprint.as_str())
        != proposal.predecessor_fingerprint.as_deref()
    {
        return Err(adapter_error(
            "curation proposal predecessor fingerprint mismatch",
        ));
    }
    let reviews =
        load_referenced_row_reviews_in(connection, &proposal.row_review_references).await?;
    proposal
        .verify_against(&report, predecessor.as_ref(), &reviews)
        .map_err(domain_error)
}

async fn insert_proposal_in(
    connection: &mut SqliteConnection,
    proposal: &CurationProposal,
) -> Result<(), QualityAdapterError> {
    sqlx::query(
        "INSERT INTO dataset_curation_proposals \
         (id, report_id, dataset_id, predecessor_id, row_review_set_fingerprint, fingerprint, \
          proposal_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(proposal.id)
    .bind(proposal.report_id)
    .bind(proposal.dataset_definition_id)
    .bind(proposal.predecessor_id)
    .bind(&proposal.row_review_set_fingerprint)
    .bind(&proposal.fingerprint)
    .bind(encode(proposal)?)
    .bind(proposal.created_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    for entry in &proposal.entries {
        sqlx::query(
            "INSERT INTO dataset_curation_proposal_entries \
             (proposal_id, source_row_id, decision, basis, fingerprint, entry_json) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(proposal.id)
        .bind(entry.source_row_id)
        .bind(curation_decision(entry.decision))
        .bind(curation_basis(entry.basis))
        .bind(&entry.fingerprint)
        .bind(encode(entry)?)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

async fn load_proposal_shallow_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<CurationProposal>, QualityAdapterError> {
    let record = sqlx::query_as::<_, ProposalRecord>(
        "SELECT id, report_id, dataset_id, predecessor_id, row_review_set_fingerprint, \
         fingerprint, proposal_json, created_at FROM dataset_curation_proposals WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let Some(record) = record else {
        return Ok(None);
    };
    let proposal: CurationProposal = decode(&record.proposal_json)?;
    if record.id != proposal.id
        || record.report_id != proposal.report_id
        || record.dataset_id != proposal.dataset_definition_id
        || record.predecessor_id != proposal.predecessor_id
        || record.row_review_set_fingerprint != proposal.row_review_set_fingerprint
        || record.fingerprint != proposal.fingerprint
        || record.created_at != proposal.created_at
        || proposal.reproduce_fingerprint().map_err(domain_error)? != proposal.fingerprint
    {
        return Err(adapter_error(
            "curation proposal normalized columns disagree with JSON",
        ));
    }
    let entries = sqlx::query(
        "SELECT source_row_id, decision, basis, fingerprint, entry_json \
         FROM dataset_curation_proposal_entries WHERE proposal_id = ? ORDER BY source_row_id",
    )
    .bind(id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    if entries.len() != proposal.entries.len() {
        return Err(adapter_error(
            "curation proposal entry manifest is incomplete",
        ));
    }
    for (record, expected) in entries.into_iter().zip(&proposal.entries) {
        let source_row_id: Uuid = record.try_get("source_row_id").map_err(sql_error)?;
        let decision: String = record.try_get("decision").map_err(sql_error)?;
        let basis: String = record.try_get("basis").map_err(sql_error)?;
        let fingerprint: String = record.try_get("fingerprint").map_err(sql_error)?;
        let entry_json: String = record.try_get("entry_json").map_err(sql_error)?;
        let value: dataset_quality_core::curation::CurationProposalEntry = decode(&entry_json)?;
        if &value != expected
            || source_row_id != expected.source_row_id
            || decision != curation_decision(expected.decision)
            || basis != curation_basis(expected.basis)
            || fingerprint != expected.fingerprint
        {
            return Err(adapter_error(
                "curation proposal entry normalized facts disagree with JSON",
            ));
        }
    }
    Ok(Some(proposal))
}

async fn load_proposal_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<CurationProposal>, QualityAdapterError> {
    let Some(proposal) = load_proposal_shallow_in(connection, id).await? else {
        return Ok(None);
    };
    validate_proposal_in(connection, &proposal).await?;
    Ok(Some(proposal))
}

fn validate_manifest_review(
    proposal: &CurationProposal,
    review: &CurationManifestReview,
) -> Result<(), QualityAdapterError> {
    if review.id.is_nil()
        || review.proposal_id != proposal.id
        || review.proposal_fingerprint != proposal.fingerprint
        || review.predecessor_id.is_some() != review.predecessor_fingerprint.is_some()
        || review.reviewer.trim().is_empty()
        || review.reason.trim().is_empty()
        || review.reproduce_fingerprint().map_err(domain_error)? != review.fingerprint
    {
        return Err(adapter_error(
            "curation manifest review immutable evidence is invalid",
        ));
    }
    Ok(())
}

fn check_manifest_review_record(
    record: ManifestReviewRecord,
) -> Result<CurationManifestReview, QualityAdapterError> {
    let review: CurationManifestReview = decode(&record.review_json)?;
    if record.id != review.id
        || record.proposal_id != review.proposal_id
        || record.predecessor_id != review.predecessor_id
        || record.decision != manifest_review_decision(review.decision)
        || record.fingerprint != review.fingerprint
        || record.created_at != review.created_at
    {
        return Err(adapter_error(
            "curation manifest review normalized columns disagree with JSON",
        ));
    }
    Ok(review)
}

async fn load_manifest_review_by_id_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<CurationManifestReview>, QualityAdapterError> {
    let record = sqlx::query_as::<_, ManifestReviewRecord>(
        "SELECT id, proposal_id, predecessor_id, decision, fingerprint, review_json, created_at \
         FROM dataset_curation_manifest_reviews WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    record.map(check_manifest_review_record).transpose()
}

async fn latest_manifest_review_in(
    connection: &mut SqliteConnection,
    proposal_id: Uuid,
) -> Result<Option<CurationManifestReview>, QualityAdapterError> {
    let record = sqlx::query_as::<_, ManifestReviewRecord>(
        "SELECT id, proposal_id, predecessor_id, decision, fingerprint, review_json, created_at \
         FROM dataset_curation_manifest_reviews WHERE proposal_id = ? \
         ORDER BY rowid DESC LIMIT 1",
    )
    .bind(proposal_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    record.map(check_manifest_review_record).transpose()
}

async fn load_manifest_reviews_in(
    connection: &mut SqliteConnection,
    proposal_id: Uuid,
) -> Result<Vec<CurationManifestReview>, QualityAdapterError> {
    let proposal = load_proposal_in(connection, proposal_id)
        .await?
        .ok_or_else(|| adapter_error("curation proposal is missing"))?;
    let records = sqlx::query_as::<_, ManifestReviewRecord>(
        "SELECT id, proposal_id, predecessor_id, decision, fingerprint, review_json, created_at \
         FROM dataset_curation_manifest_reviews WHERE proposal_id = ? ORDER BY rowid",
    )
    .bind(proposal_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let mut values = Vec::with_capacity(records.len());
    let mut latest: Option<(Uuid, String)> = None;
    for record in records {
        let review = check_manifest_review_record(record)?;
        validate_manifest_review(&proposal, &review)?;
        require_review_predecessor(
            review.predecessor_id,
            review.predecessor_fingerprint.as_deref(),
            latest
                .as_ref()
                .map(|(id, fingerprint)| (*id, fingerprint.as_str())),
            "manifest review",
        )?;
        latest = Some((review.id, review.fingerprint.clone()));
        values.push(review);
    }
    Ok(values)
}

async fn validate_manifest_in(
    connection: &mut SqliteConnection,
    manifest: &ApprovedCurationManifest,
) -> Result<(), QualityAdapterError> {
    let report = load_report_in(connection, manifest.report_id)
        .await?
        .ok_or_else(|| adapter_error("curation manifest report is missing"))?;
    let proposal = load_proposal_in(connection, manifest.proposal_id)
        .await?
        .ok_or_else(|| adapter_error("curation manifest proposal is missing"))?;
    let predecessor = match proposal.predecessor_id {
        Some(id) => Some(
            load_proposal_shallow_in(connection, id)
                .await?
                .ok_or_else(|| adapter_error("curation proposal predecessor is missing"))?,
        ),
        None => None,
    };
    let row_reviews =
        load_referenced_row_reviews_in(connection, &proposal.row_review_references).await?;
    let reviews = load_manifest_reviews_in(connection, proposal.id).await?;
    manifest
        .verify_against(
            &report,
            &proposal,
            predecessor.as_ref(),
            &row_reviews,
            &reviews,
        )
        .map(|_| ())
        .map_err(domain_error)
}

async fn insert_manifest_in(
    connection: &mut SqliteConnection,
    manifest: &ApprovedCurationManifest,
) -> Result<(), QualityAdapterError> {
    sqlx::query(
        "INSERT INTO dataset_curation_manifests \
         (id, report_id, proposal_id, approval_id, dataset_id, complete_member_fingerprint, \
          selected_member_fingerprint, fingerprint, manifest_json, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(manifest.id)
    .bind(manifest.report_id)
    .bind(manifest.proposal_id)
    .bind(manifest.approval_id)
    .bind(manifest.dataset_definition_id)
    .bind(&manifest.complete_member_fingerprint)
    .bind(&manifest.selected_member_fingerprint)
    .bind(&manifest.fingerprint)
    .bind(encode(manifest)?)
    .bind(manifest.created_at)
    .execute(&mut *connection)
    .await
    .map_err(sql_error)?;
    for member in &manifest.members {
        sqlx::query(
            "INSERT INTO dataset_curation_manifest_members \
             (manifest_id, source_row_id, disposition, source_row_fingerprint, \
              proposal_entry_fingerprint, member_json) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(manifest.id)
        .bind(member.source_row_id)
        .bind(curation_disposition(member.disposition))
        .bind(&member.source_row_fingerprint)
        .bind(&member.proposal_entry_fingerprint)
        .bind(encode(member)?)
        .execute(&mut *connection)
        .await
        .map_err(sql_error)?;
    }
    Ok(())
}

async fn load_manifest_shallow_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<ApprovedCurationManifest>, QualityAdapterError> {
    let record = sqlx::query_as::<_, ManifestRecord>(
        "SELECT id, report_id, proposal_id, approval_id, dataset_id, \
         complete_member_fingerprint, selected_member_fingerprint, fingerprint, \
         manifest_json, created_at FROM dataset_curation_manifests WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    let Some(record) = record else {
        return Ok(None);
    };
    let manifest: ApprovedCurationManifest = decode(&record.manifest_json)?;
    if record.id != manifest.id
        || record.report_id != manifest.report_id
        || record.proposal_id != manifest.proposal_id
        || record.approval_id != manifest.approval_id
        || record.dataset_id != manifest.dataset_definition_id
        || record.complete_member_fingerprint != manifest.complete_member_fingerprint
        || record.selected_member_fingerprint != manifest.selected_member_fingerprint
        || record.fingerprint != manifest.fingerprint
        || record.created_at != manifest.created_at
        || manifest.reproduce_fingerprint().map_err(domain_error)? != manifest.fingerprint
    {
        return Err(adapter_error(
            "curation manifest normalized columns disagree with JSON",
        ));
    }
    let members = sqlx::query(
        "SELECT source_row_id, disposition, source_row_fingerprint, \
         proposal_entry_fingerprint, member_json FROM dataset_curation_manifest_members \
         WHERE manifest_id = ? ORDER BY source_row_id",
    )
    .bind(id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    if members.len() != manifest.members.len() {
        return Err(adapter_error(
            "curation manifest normalized membership is incomplete",
        ));
    }
    for (record, expected) in members.into_iter().zip(&manifest.members) {
        let source_row_id: Uuid = record.try_get("source_row_id").map_err(sql_error)?;
        let disposition: String = record.try_get("disposition").map_err(sql_error)?;
        let source_row_fingerprint: String = record
            .try_get("source_row_fingerprint")
            .map_err(sql_error)?;
        let proposal_entry_fingerprint: String = record
            .try_get("proposal_entry_fingerprint")
            .map_err(sql_error)?;
        let member_json: String = record.try_get("member_json").map_err(sql_error)?;
        let value: dataset_quality_core::curation::CurationManifestMember = decode(&member_json)?;
        if &value != expected
            || source_row_id != expected.source_row_id
            || disposition != curation_disposition(expected.disposition)
            || source_row_fingerprint != expected.source_row_fingerprint
            || proposal_entry_fingerprint != expected.proposal_entry_fingerprint
        {
            return Err(adapter_error(
                "curation manifest member normalized facts disagree with JSON",
            ));
        }
    }
    Ok(Some(manifest))
}

async fn load_manifest_in(
    connection: &mut SqliteConnection,
    id: Uuid,
) -> Result<Option<ApprovedCurationManifest>, QualityAdapterError> {
    let Some(manifest) = load_manifest_shallow_in(connection, id).await? else {
        return Ok(None);
    };
    validate_manifest_in(connection, &manifest).await?;
    Ok(Some(manifest))
}

async fn require_current_manifest_for_new_application_in(
    connection: &mut SqliteConnection,
    manifest: &ApprovedCurationManifest,
) -> Result<(), QualityAdapterError> {
    let latest_proposal_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM dataset_curation_proposals WHERE report_id = ? \
         ORDER BY rowid DESC LIMIT 1",
    )
    .bind(manifest.report_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?;
    if latest_proposal_id != Some(manifest.proposal_id) {
        return Err(adapter_error(
            "approved curation manifest is stale because a newer proposal exists",
        ));
    }

    let proposal = load_proposal_in(connection, manifest.proposal_id)
        .await?
        .ok_or_else(|| adapter_error("curation application proposal is missing"))?;
    let mut current_review_references = load_row_reviews_in(connection, manifest.report_id)
        .await?
        .into_iter()
        .map(|review| ArtifactReference {
            id: review.id,
            fingerprint: review.fingerprint,
        })
        .collect::<Vec<_>>();
    current_review_references.sort();
    if proposal.row_review_references != current_review_references {
        return Err(adapter_error(
            "approved curation manifest is stale because newer row reviews exist",
        ));
    }
    Ok(())
}

fn validate_snapshot(
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
) -> Result<(), QualityAdapterError> {
    dataset_core::splitting::verify_snapshot(snapshot, members)
        .map_err(|error| adapter_error(error.to_string()))
}

async fn validate_application_in(
    connection: &mut SqliteConnection,
    application: &CurationApplication,
    manifest: &ApprovedCurationManifest,
    snapshot: &DatasetSnapshot,
    members: &[SnapshotMember],
) -> Result<(), QualityAdapterError> {
    validate_snapshot(snapshot, members)?;
    let report = load_report_in(connection, manifest.report_id)
        .await?
        .ok_or_else(|| adapter_error("curation application report is missing"))?;
    let proposal = load_proposal_in(connection, manifest.proposal_id)
        .await?
        .ok_or_else(|| adapter_error("curation application proposal is missing"))?;
    let predecessor = match proposal.predecessor_id {
        Some(id) => Some(
            load_proposal_shallow_in(connection, id)
                .await?
                .ok_or_else(|| adapter_error("curation proposal predecessor is missing"))?,
        ),
        None => None,
    };
    let row_reviews =
        load_referenced_row_reviews_in(connection, &proposal.row_review_references).await?;
    let manifest_reviews = load_manifest_reviews_in(connection, proposal.id).await?;
    application
        .verify_against(
            manifest,
            &report,
            &proposal,
            predecessor.as_ref(),
            &row_reviews,
            &manifest_reviews,
            snapshot,
            members,
        )
        .map_err(domain_error)
}

async fn verify_snapshot_sources_in(
    connection: &mut SqliteConnection,
    manifest: &ApprovedCurationManifest,
    members: &[SnapshotMember],
) -> Result<(), QualityAdapterError> {
    let ids = members
        .iter()
        .map(|member| member.source_row_id)
        .collect::<Vec<_>>();
    let rows = load_source_rows_in(connection, manifest.dataset_definition_id, &ids).await?;
    let by_id = rows
        .into_iter()
        .map(|row| (row.id, row))
        .collect::<BTreeMap<_, _>>();
    for member in members {
        let source = &by_id[&member.source_row_id];
        if member.text != source.text
            || member.label != source.label
            || member.dimensions != source.dimensions
            || member.fields != source.fields
            || member.source_provenance != source.provenance
            || member.source_created_at != source.created_at
        {
            return Err(adapter_error(
                "curated snapshot member differs from its authoritative source row",
            ));
        }
    }
    Ok(())
}

fn check_application_record(
    record: ApplicationRecord,
) -> Result<CurationApplication, QualityAdapterError> {
    let application: CurationApplication = decode(&record.application_json)?;
    if record.id != application.id
        || record.manifest_id != application.manifest_id
        || record.approval_id != application.approval_id
        || record.snapshot_id != application.snapshot_id
        || record.manifest_fingerprint != application.manifest_fingerprint
        || record.snapshot_fingerprint != application.snapshot_fingerprint
        || record.selected_member_fingerprint != application.selected_member_fingerprint
        || record.snapshot_membership_fingerprint != application.snapshot_membership_fingerprint
        || record.fingerprint != application.fingerprint
        || record.applied_at != application.applied_at
        || application.reproduce_fingerprint().map_err(domain_error)? != application.fingerprint
    {
        return Err(adapter_error(
            "curation application normalized columns disagree with JSON",
        ));
    }
    Ok(application)
}

async fn load_application_by_any_identity_in(
    connection: &mut SqliteConnection,
    id: Uuid,
    manifest_id: Uuid,
    snapshot_id: Uuid,
) -> Result<Option<CurationApplication>, QualityAdapterError> {
    let records = sqlx::query_as::<_, ApplicationRecord>(
        "SELECT id, manifest_id, approval_id, snapshot_id, manifest_fingerprint, \
         snapshot_fingerprint, selected_member_fingerprint, snapshot_membership_fingerprint, \
         fingerprint, application_json, applied_at FROM dataset_curation_applications \
         WHERE id = ? OR manifest_id = ? OR snapshot_id = ?",
    )
    .bind(id)
    .bind(manifest_id)
    .bind(snapshot_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    if records.len() > 1 {
        return Err(adapter_error(
            "curation application identities resolve to conflicting persisted records",
        ));
    }
    records
        .into_iter()
        .next()
        .map(check_application_record)
        .transpose()
}

async fn load_snapshot_bundle_in(
    connection: &mut SqliteConnection,
    snapshot_id: Uuid,
) -> Result<(DatasetSnapshot, Vec<SnapshotMember>), QualityAdapterError> {
    let record = sqlx::query_as::<_, SnapshotRecord>(
        "SELECT id, source_dataset_id, name, description, split_configuration_json, \
         member_count, fingerprint, created_at FROM dataset_snapshots WHERE id = ?",
    )
    .bind(snapshot_id)
    .fetch_optional(&mut *connection)
    .await
    .map_err(sql_error)?
    .ok_or_else(|| adapter_error("curated dataset snapshot is missing"))?;
    let snapshot = record
        .into_domain()
        .map_err(|error| adapter_error(error.to_string()))?;
    let records = sqlx::query_as::<_, SnapshotMemberRecord>(
        "SELECT id, snapshot_id, source_row_id, split, text, label, dimensions_json, fields_json, \
         source_provenance_json, source_created_at FROM dataset_snapshot_members \
         WHERE snapshot_id = ? ORDER BY source_row_id",
    )
    .bind(snapshot_id)
    .fetch_all(&mut *connection)
    .await
    .map_err(sql_error)?;
    let members = records
        .into_iter()
        .map(|record| {
            record
                .into_domain()
                .map_err(|error| adapter_error(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_snapshot(&snapshot, &members)?;
    Ok((snapshot, members))
}

async fn validate_persisted_application_in(
    connection: &mut SqliteConnection,
    application: &CurationApplication,
) -> Result<(), QualityAdapterError> {
    let manifest = load_manifest_in(connection, application.manifest_id)
        .await?
        .ok_or_else(|| adapter_error("curation application manifest is missing"))?;
    let (snapshot, members) = load_snapshot_bundle_in(connection, application.snapshot_id).await?;
    validate_application_in(connection, application, &manifest, &snapshot, &members).await?;
    verify_snapshot_sources_in(connection, &manifest, &members).await
}

fn require_review_predecessor(
    predecessor_id: Option<Uuid>,
    predecessor_fingerprint: Option<&str>,
    latest: Option<(Uuid, &str)>,
    artifact: &str,
) -> Result<(), QualityAdapterError> {
    let supplied = predecessor_id.zip(predecessor_fingerprint);
    if supplied == latest {
        Ok(())
    } else {
        Err(adapter_error(format!(
            "{artifact} predecessor is stale; reviews are append-only"
        )))
    }
}

const fn audit_selection(value: AuditSelection) -> &'static str {
    match value {
        AuditSelection::Selected => "selected",
        AuditSelection::UnselectedReportOnly => "unselected_report_only",
    }
}

const fn run_state(value: QualityAuditRunState) -> &'static str {
    match value {
        QualityAuditRunState::Queued => "queued",
        QualityAuditRunState::Running => "running",
        QualityAuditRunState::Completed => "completed",
        QualityAuditRunState::Failed => "failed",
        QualityAuditRunState::Cancelled => "cancelled",
    }
}

const fn attempt_state(value: EvaluatorAttemptState) -> &'static str {
    match value {
        EvaluatorAttemptState::Started => "started",
        EvaluatorAttemptState::Succeeded => "succeeded",
        EvaluatorAttemptState::Failed => "failed",
        EvaluatorAttemptState::InvalidResponse => "invalid_response",
        EvaluatorAttemptState::Interrupted => "interrupted",
    }
}

const fn verdict(value: QualityVerdict) -> &'static str {
    match value {
        QualityVerdict::Qualified => "qualified",
        QualityVerdict::Borderline => "borderline",
        QualityVerdict::Quarantined => "quarantined",
    }
}

const fn report_row_verdict(value: ReportRowVerdict) -> &'static str {
    match value {
        ReportRowVerdict::Qualified => "qualified",
        ReportRowVerdict::Borderline => "borderline",
        ReportRowVerdict::Quarantined => "quarantined",
        ReportRowVerdict::InvalidEvaluatorOutput => "invalid_evaluator_output",
        ReportRowVerdict::Unaudited(_) => "unaudited",
    }
}

const fn row_review_decision(value: RowQualityReviewDecision) -> &'static str {
    match value {
        RowQualityReviewDecision::Include => "include",
        RowQualityReviewDecision::Exclude => "exclude",
        RowQualityReviewDecision::RequestReassessment => "request_reassessment",
    }
}

const fn curation_decision(value: CurationDecision) -> &'static str {
    match value {
        CurationDecision::Include => "include",
        CurationDecision::Exclude => "exclude",
        CurationDecision::NeedsReview => "needs_review",
    }
}

const fn curation_basis(value: CurationDecisionBasis) -> &'static str {
    match value {
        CurationDecisionBasis::QualifiedAssessment => "qualified_assessment",
        CurationDecisionBasis::QuarantinedAssessment => "quarantined_assessment",
        CurationDecisionBasis::InvalidEvaluatorOutput => "invalid_evaluator_output",
        CurationDecisionBasis::Unaudited => "unaudited",
        CurationDecisionBasis::ConflictingEvidence => "conflicting_evidence",
        CurationDecisionBasis::BorderlineAssessment => "borderline_assessment",
        CurationDecisionBasis::HumanIncludeOverride => "human_include_override",
        CurationDecisionBasis::HumanExclude => "human_exclude",
        CurationDecisionBasis::ReassessmentRequested => "reassessment_requested",
    }
}

const fn manifest_review_decision(value: CurationManifestReviewDecision) -> &'static str {
    match value {
        CurationManifestReviewDecision::Approve => "approve",
        CurationManifestReviewDecision::Reject => "reject",
        CurationManifestReviewDecision::RequestRevision => "request_revision",
    }
}

const fn curation_disposition(value: CurationDisposition) -> &'static str {
    match value {
        CurationDisposition::Include => "include",
        CurationDisposition::Exclude => "exclude",
    }
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String, QualityAdapterError> {
    serde_json::to_string(value).map_err(|error| adapter_error(error.to_string()))
}

fn decode<T: serde::de::DeserializeOwned>(value: &str) -> Result<T, QualityAdapterError> {
    serde_json::from_str(value).map_err(|error| adapter_error(error.to_string()))
}

fn as_i64(value: u64) -> Result<i64, QualityAdapterError> {
    i64::try_from(value).map_err(|error| adapter_error(error.to_string()))
}

fn as_u64(value: i64) -> Result<u64, QualityAdapterError> {
    u64::try_from(value).map_err(|error| adapter_error(error.to_string()))
}

fn as_u32(value: i64) -> Result<u32, QualityAdapterError> {
    u32::try_from(value).map_err(|error| adapter_error(error.to_string()))
}

fn require_one(rows: u64, artifact: &str) -> Result<(), QualityAdapterError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(adapter_error(format!(
            "{artifact} was not found or changed"
        )))
    }
}

fn sql_error(error: sqlx::Error) -> QualityAdapterError {
    adapter_error(error.to_string())
}

fn domain_error(error: QualityError) -> QualityAdapterError {
    adapter_error(error.to_string())
}

fn adapter_error(message: impl Into<String>) -> QualityAdapterError {
    QualityAdapterError(message.into())
}

#[cfg(test)]
tokio::task_local! {
    static FULL_EVIDENCE_LOADS: std::cell::Cell<usize>;
    static FULL_PLAN_LOADS: std::cell::Cell<usize>;
    static SINGLE_ASSESSMENT_HYDRATIONS: std::cell::Cell<usize>;
    static SOURCE_ROW_DECODES: std::cell::Cell<usize>;
}

#[cfg(test)]
fn note_full_evidence_load() {
    let _ = FULL_EVIDENCE_LOADS.try_with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn note_full_plan_load() {
    let _ = FULL_PLAN_LOADS.try_with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn note_single_assessment_hydration() {
    let _ = SINGLE_ASSESSMENT_HYDRATIONS.try_with(|count| count.set(count.get() + 1));
}

#[cfg(test)]
fn note_source_row_decode() {
    let _ = SOURCE_ROW_DECODES.try_with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn note_full_evidence_load() {}

#[cfg(not(test))]
fn note_full_plan_load() {}

#[cfg(not(test))]
fn note_single_assessment_hydration() {}

#[cfg(not(test))]
fn note_source_row_decode() {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{TimeZone, Utc};
    use dataset_core::{
        domain::{SnapshotSplit, SourceProvenance, SplitConfiguration, SplitRatios},
        ports::SnapshotStore,
        splitting::build_snapshot,
    };
    use dataset_quality_core::{
        assessment::{
            BlindEvaluatorRequest, EvaluatorExecutionLocation, EvaluatorGuidance,
            EvaluatorIdentity, EvaluatorIndependence, EvaluatorRequestBudget, RowAssessmentDraft,
            RowQualityAssessment,
        },
        curation::{
            ApprovedCurationManifest, CurationApplication, CurationManifestReview,
            CurationManifestReviewDecision, CurationProposal, DatasetQualityReport,
            RowQualityReview, RowQualityReviewDecision,
        },
        lifecycle::{AuditProgress, EvaluatorAttempt, ProviderUsage, QualityAuditRun},
        policy::{
            AuditMode, BasisPoints, EvaluatorEgressPolicy, QualityPolicyPresetControls,
            QualityPreset,
        },
        population::{AuditPlan, GuidanceReferences},
        ports::{DatasetQualityStore, QualityCandidateSource},
    };
    use generation_core::domain::{DatasetDefinition, DimensionDefinition};
    use workflow_core::{
        governance::{
            CohortDisposition, CohortOrigin, CohortRole, CohortRoleDecision, EvaluationCohort,
        },
        ports::GovernanceStore,
    };

    use super::*;
    use crate::{SqliteStore, insert_dataset};

    struct CurationFixture {
        store: SqliteStore,
        dataset: DatasetDefinition,
        rows: Vec<SourceRow>,
        plan: AuditPlan,
        run: QualityAuditRun,
        request: BlindEvaluatorRequest,
        attempt: EvaluatorAttempt,
        report: DatasetQualityReport,
        proposal: CurationProposal,
        approval: CurationManifestReview,
        manifest: ApprovedCurationManifest,
        snapshot: DatasetSnapshot,
        members: Vec<SnapshotMember>,
        application: CurationApplication,
    }

    struct AttemptFixture {
        store: SqliteStore,
        dataset: DatasetDefinition,
        rows: Vec<SourceRow>,
        plan: AuditPlan,
        run: QualityAuditRun,
        request: BlindEvaluatorRequest,
        attempt: EvaluatorAttempt,
        lease: AuditExecutionLease,
    }

    struct EligibilityFixture {
        store: SqliteStore,
        dataset: DatasetDefinition,
        rows: Vec<SourceRow>,
        snapshot: DatasetSnapshot,
        members: Vec<SnapshotMember>,
    }

    async fn eligibility_fixture() -> EligibilityFixture {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("temporary store");
        let dataset = DatasetDefinition::with_identity(
            Uuid::from_u128(20),
            "eligibility",
            "Classify support requests",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 2, 1, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("dataset");
        let mut rows = (501..=506)
            .map(|id| {
                source_row(
                    dataset.id,
                    id,
                    &format!("billing candidate {id}"),
                    "billing",
                    "easy",
                )
            })
            .collect::<Vec<_>>();
        rows[5].provenance = SourceProvenance::Generated {
            generation_job_id: Uuid::from_u128(600),
            backend: "fake".into(),
            model: "fake-v1".into(),
            construction_plan_fingerprint: None,
        };
        persist_source_fixture(&store, &dataset, &rows).await;
        let (snapshot, members) = build_snapshot(
            dataset.id,
            "governed-source",
            None,
            SplitConfiguration::new(SplitRatios::new(0.5, 0.5, 0.0).expect("split ratios"), 37),
            rows.clone(),
        )
        .expect("snapshot");
        SnapshotStore::create_snapshot(&store, &snapshot, &members)
            .await
            .expect("persist snapshot");
        assert!(
            members
                .iter()
                .any(|member| member.split == SnapshotSplit::Train)
        );
        assert!(
            members
                .iter()
                .any(|member| member.split == SnapshotSplit::Validation)
        );
        EligibilityFixture {
            store,
            dataset,
            rows,
            snapshot,
            members,
        }
    }

    async fn create_governed_cohort(
        store: &SqliteStore,
        snapshot: &DatasetSnapshot,
        split: SnapshotSplit,
        origin: CohortOrigin,
        role: CohortRole,
    ) -> (EvaluationCohort, CohortRoleDecision) {
        let cohort = EvaluationCohort::new(
            format!("{origin:?}-{role:?}-{split:?}"),
            snapshot.id,
            snapshot.fingerprint.clone(),
            split,
            origin,
        )
        .expect("cohort");
        let decision = CohortRoleDecision::initial(&cohort, role, "fixture governance role")
            .expect("initial role");
        GovernanceStore::create_cohort(store, &cohort, &decision)
            .await
            .expect("persist cohort");
        (cohort, decision)
    }

    async fn attempt_fixture() -> AttemptFixture {
        let store = SqliteStore::connect("sqlite::memory:")
            .await
            .expect("temporary store");
        let dataset = DatasetDefinition::with_identity(
            Uuid::from_u128(10),
            "support",
            "Classify support requests",
            vec!["billing".into(), "fraud".into()],
            vec![
                DimensionDefinition::new("difficulty", vec!["easy".into(), "hard".into()])
                    .expect("dimension"),
            ],
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("dataset");
        let rows = vec![
            source_row(dataset.id, 101, "charged twice", "billing", "easy"),
            source_row(dataset.id, 102, "card was stolen", "fraud", "hard"),
        ];
        persist_source_fixture(&store, &dataset, &rows).await;

        let policy = QualityPreset::Fast
            .compile(QualityPolicyPresetControls {
                audit_mode: AuditMode::FullPopulation,
                egress_policy: EvaluatorEgressPolicy::LocalOnly,
                evaluate_authenticity: false,
                maximum_cost_microusd: Some(10_000),
            })
            .expect("policy");
        let guidance = EvaluatorGuidance::default();
        let resolved_guidance_fingerprint = guidance
            .reproduce_fingerprint()
            .expect("guidance fingerprint");
        let plan = AuditPlan::with_identity(
            Uuid::from_u128(200),
            &dataset,
            policy,
            GuidanceReferences::default(),
            resolved_guidance_fingerprint.clone(),
            "quality-evaluator-v1",
            rows.clone(),
            Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0)
                .single()
                .expect("time"),
        )
        .expect("plan");
        let evaluator = EvaluatorIdentity::new(
            "fake-quality",
            "fixture-v1",
            "quality-evaluator-v1",
            "sha256:fake-quality-config",
            EvaluatorIndependence::Primary,
            EvaluatorExecutionLocation::LocalProcess,
        )
        .expect("evaluator");
        let mut run = QualityAuditRun::queue(&plan, evaluator.clone(), Vec::new()).expect("run");
        store
            .create_audit(&plan, &run, &guidance)
            .await
            .expect("create audit");
        run.start(&plan).expect("start run");
        store.save_audit_run(&run).await.expect("save running run");
        let request = BlindEvaluatorRequest::create(
            &plan,
            Uuid::from_u128(301),
            run.id,
            Uuid::from_u128(302),
            1,
            1,
            &evaluator,
            rows.clone(),
            guidance,
            resolved_guidance_fingerprint,
            EvaluatorRequestBudget {
                maximum_input_tokens: 10_000,
                maximum_output_tokens: 10_000,
                maximum_total_tokens: 20_000,
                maximum_cost_microusd: Some(1_000),
            },
        )
        .expect("request");
        let attempt = EvaluatorAttempt::start(&plan, &run, &request, evaluator).expect("attempt");
        run.reserve_attempt(&plan, &request, &attempt.evaluator)
            .expect("reserve attempt");
        let lease = store
            .acquire_audit_execution_lease(run.id, Uuid::new_v4())
            .await
            .expect("acquire fixture execution lease");
        AttemptFixture {
            store,
            dataset,
            rows,
            plan,
            run,
            request,
            attempt,
            lease,
        }
    }

    async fn fixture() -> CurationFixture {
        let AttemptFixture {
            store,
            dataset,
            rows,
            plan,
            mut run,
            request,
            attempt,
            lease,
        } = attempt_fixture().await;
        let checked = CheckedAuditPlan::new(&plan).expect("checked plan");
        store
            .record_attempt(&checked, &lease, &attempt, &request, &run)
            .await
            .expect("record attempt and request");
        let mut terminal_attempt = attempt.clone();
        terminal_attempt
            .invalidate(
                ProviderUsage::default(),
                rows.iter().map(|row| row.id).collect(),
                "fixture evaluator returned invalid output",
                serde_json::Value::Null,
            )
            .expect("invalidate attempt");
        run.reconcile_progress(AuditProgress {
            population_rows: 2,
            selected_rows: 2,
            invalid_rows: 2,
            ..AuditProgress::default()
        })
        .expect("invalid rows");
        run.complete(&plan).expect("complete run");
        store
            .finish_attempt(&checked, &lease, &terminal_attempt, &[], &run)
            .await
            .expect("finish invalid attempt and audit");
        store
            .release_audit_execution_lease(&lease)
            .await
            .expect("release fixture execution lease");
        let report = DatasetQualityReport::create(
            &plan,
            &run,
            std::slice::from_ref(&request),
            std::slice::from_ref(&terminal_attempt),
            &[],
        )
        .expect("report");
        store.save_report(&report).await.expect("save report");

        let row_reviews = rows
            .iter()
            .map(|row| {
                RowQualityReview::create(
                    &report,
                    row.id,
                    None,
                    RowQualityReviewDecision::Include,
                    "operator",
                    "verified fixture row manually",
                )
                .expect("row review")
            })
            .collect::<Vec<_>>();
        for review in &row_reviews {
            store
                .append_row_review(review)
                .await
                .expect("append row review");
        }
        let proposal = CurationProposal::create(&report, None, &row_reviews).expect("proposal");
        store
            .save_curation_proposal(&proposal)
            .await
            .expect("save proposal");
        let approval = CurationManifestReview::create(
            &proposal,
            None,
            CurationManifestReviewDecision::Approve,
            "operator",
            "approve exact curated selection",
        )
        .expect("approval");
        store
            .append_manifest_review(&approval)
            .await
            .expect("append approval");
        let manifest = ApprovedCurationManifest::create(
            &report,
            &proposal,
            None,
            &row_reviews,
            std::slice::from_ref(&approval),
        )
        .expect("manifest");
        store.save_manifest(&manifest).await.expect("save manifest");
        let (snapshot, members) = build_snapshot(
            dataset.id,
            "qualified-v1",
            Some("curated fixture".into()),
            SplitConfiguration::new(SplitRatios::new(0.5, 0.5, 0.0).expect("ratios"), 42),
            rows.clone(),
        )
        .expect("snapshot");
        let application = CurationApplication::create(
            &manifest,
            &report,
            &proposal,
            None,
            &row_reviews,
            std::slice::from_ref(&approval),
            &snapshot,
            &members,
        )
        .expect("application");
        CurationFixture {
            store,
            dataset,
            rows,
            plan,
            run,
            request,
            attempt: terminal_attempt,
            report,
            proposal,
            approval,
            manifest,
            snapshot,
            members,
            application,
        }
    }

    fn source_row(
        dataset_id: Uuid,
        id: u128,
        text: &str,
        label: &str,
        difficulty: &str,
    ) -> SourceRow {
        SourceRow {
            id: Uuid::from_u128(id),
            dataset_id,
            text: text.into(),
            label: label.into(),
            dimensions: BTreeMap::from([("difficulty".into(), difficulty.into())]),
            fields: BTreeMap::new(),
            provenance: SourceProvenance::Imported {
                import_id: Uuid::from_u128(99),
                source_path: "fixture.jsonl".into(),
                source_row_number: id as u64,
            },
            created_at: Utc
                .with_ymd_and_hms(2026, 1, 1, 0, 0, (id % 60) as u32)
                .single()
                .expect("time"),
        }
    }

    async fn persist_source_fixture(
        store: &SqliteStore,
        dataset: &DatasetDefinition,
        rows: &[SourceRow],
    ) {
        let mut transaction = store.pool().begin().await.expect("transaction");
        insert_dataset(&mut transaction, dataset)
            .await
            .expect("insert dataset");
        for row in rows {
            let source_kind = match row.provenance {
                SourceProvenance::Generated { .. } => "generated",
                SourceProvenance::Imported { .. } => "imported",
            };
            sqlx::query(
                "INSERT INTO dataset_source_rows \
                 (id, dataset_id, source_kind, source_ref, cell_key, text, normalized_text, \
                  label, dimensions_json, provenance_json, created_at, fields_json) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(row.id)
            .bind(row.dataset_id)
            .bind(source_kind)
            .bind(row.id)
            .bind(format!("{}|{}", row.label, row.dimensions["difficulty"]))
            .bind(&row.text)
            .bind(row.text.to_lowercase())
            .bind(&row.label)
            .bind(serde_json::to_string(&row.dimensions).expect("dimensions"))
            .bind(serde_json::to_string(&row.provenance).expect("provenance"))
            .bind(row.created_at)
            .bind(serde_json::to_string(&row.fields).expect("fields"))
            .execute(&mut *transaction)
            .await
            .expect("source row");
        }
        transaction.commit().await.expect("commit fixture");
    }

    fn successful_assessments(
        fixture: &AttemptFixture,
        request: &BlindEvaluatorRequest,
    ) -> Vec<RowQualityAssessment> {
        request
            .rows
            .iter()
            .map(|requested| {
                let source = fixture
                    .rows
                    .iter()
                    .find(|row| row.id == requested.source_row_id)
                    .expect("request source row");
                let item = fixture
                    .plan
                    .items
                    .iter()
                    .find(|item| item.source_row_id == source.id)
                    .expect("plan item");
                let label_scores = fixture
                    .plan
                    .dataset_schema
                    .labels
                    .iter()
                    .map(|label| {
                        (
                            label.clone(),
                            BasisPoints::new(if label == &source.label { 9_000 } else { 1_000 })
                                .expect("basis points"),
                        )
                    })
                    .collect();
                let dimension_scores = fixture
                    .plan
                    .dataset_schema
                    .dimensions
                    .iter()
                    .map(|dimension| {
                        let assigned = &source.dimensions[&dimension.name];
                        (
                            dimension.name.clone(),
                            dimension
                                .values
                                .iter()
                                .map(|value| {
                                    (
                                        value.clone(),
                                        BasisPoints::new(if value == assigned {
                                            9_000
                                        } else {
                                            1_000
                                        })
                                        .expect("basis points"),
                                    )
                                })
                                .collect(),
                        )
                    })
                    .collect();
                RowQualityAssessment::create(
                    &fixture.plan,
                    item,
                    request,
                    fixture.attempt.evaluator.clone(),
                    RowAssessmentDraft {
                        source_row_id: source.id,
                        source_row_fingerprint: requested.source_row_fingerprint.clone(),
                        label_scores,
                        dimension_scores,
                        authenticity_score: None,
                        label_leakage_risk: BasisPoints::new(0).expect("basis points"),
                        shortcut_risk: BasisPoints::new(0).expect("basis points"),
                        confidence: BasisPoints::new(9_000).expect("basis points"),
                        issue_codes: Vec::new(),
                        rationale: "deterministic storage fixture".into(),
                    },
                    &[],
                    Utc::now(),
                )
                .expect("assessment")
            })
            .collect()
    }

    #[tokio::test]
    async fn qualification_artifacts_round_trip_and_application_is_idempotent() {
        let fixture = fixture().await;
        assert_eq!(
            fixture
                .store
                .get_evaluator_request(fixture.request.id)
                .await
                .expect("load request"),
            Some(fixture.request.clone())
        );
        assert_eq!(
            fixture
                .store
                .list_evaluator_requests(fixture.run.id)
                .await
                .expect("list requests"),
            vec![fixture.request.clone()]
        );
        assert_eq!(
            fixture
                .store
                .list_attempts(fixture.run.id)
                .await
                .expect("list attempts"),
            vec![fixture.attempt.clone()]
        );
        assert_eq!(
            fixture
                .store
                .get_audit_plan(fixture.plan.id)
                .await
                .expect("load plan"),
            Some(fixture.plan.clone())
        );
        assert_eq!(
            fixture
                .store
                .get_audit_guidance(fixture.plan.id)
                .await
                .expect("load pinned guidance"),
            Some(EvaluatorGuidance::default())
        );
        fixture
            .store
            .apply_manifest(&fixture.application, &fixture.snapshot, &fixture.members)
            .await
            .expect("apply manifest");
        fixture
            .store
            .apply_manifest(&fixture.application, &fixture.snapshot, &fixture.members)
            .await
            .expect("exact idempotent replay");

        let loaded = fixture
            .store
            .curation_application_for_snapshot(fixture.snapshot.id)
            .await
            .expect("load application")
            .expect("application exists");
        assert_eq!(loaded, fixture.application);
        assert_eq!(
            fixture
                .store
                .curation_application_for_manifest(fixture.manifest.id)
                .await
                .expect("load application by manifest"),
            Some(fixture.application.clone())
        );
        assert_eq!(
            fixture
                .store
                .get_manifest(fixture.manifest.id)
                .await
                .expect("load manifest"),
            Some(fixture.manifest.clone())
        );
        assert_eq!(
            fixture
                .store
                .manifest_for_proposal(fixture.proposal.id)
                .await
                .expect("load manifest by proposal"),
            Some(fixture.manifest)
        );
        assert_eq!(
            fixture
                .store
                .get_curation_proposal(fixture.proposal.id)
                .await
                .expect("load proposal"),
            Some(fixture.proposal.clone())
        );
        assert_eq!(
            fixture
                .store
                .latest_curation_proposal(fixture.report.id)
                .await
                .expect("load latest proposal"),
            Some(fixture.proposal)
        );
        assert_eq!(
            fixture
                .store
                .get_report(fixture.report.id)
                .await
                .expect("load report"),
            Some(fixture.report)
        );
        let foreign_key_violations: Vec<(String, i64, String, i64)> =
            sqlx::query_as("PRAGMA foreign_key_check")
                .fetch_all(fixture.store.pool())
                .await
                .expect("foreign key check");
        assert!(foreign_key_violations.is_empty());
    }

    #[tokio::test]
    async fn candidate_source_rejects_partial_or_duplicate_identity_sets() {
        let fixture = fixture().await;
        let mut expected = fixture.rows.clone();
        expected.sort_by_key(|row| row.id);
        assert_eq!(
            fixture
                .store
                .list_source_rows(fixture.dataset.id)
                .await
                .expect("all source rows"),
            expected
        );
        assert!(
            fixture
                .store
                .get_source_rows(
                    fixture.dataset.id,
                    vec![fixture.rows[0].id, fixture.rows[0].id],
                )
                .await
                .is_err()
        );
        assert!(
            fixture
                .store
                .get_source_rows(fixture.dataset.id, vec![Uuid::new_v4()])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn candidate_source_keeps_ordinary_generated_and_imported_rows() {
        let fixture = eligibility_fixture().await;
        let mut expected = fixture.rows.clone();
        expected.sort_by_key(|row| row.id);
        let eligible = fixture
            .store
            .list_source_rows(fixture.dataset.id)
            .await
            .expect("ordinary candidates");
        assert_eq!(eligible, expected);
        assert!(
            eligible
                .iter()
                .any(|row| matches!(&row.provenance, SourceProvenance::Generated { .. }))
        );
        assert!(
            eligible
                .iter()
                .any(|row| matches!(&row.provenance, SourceProvenance::Imported { .. }))
        );
    }

    #[tokio::test]
    async fn candidate_source_withholds_every_row_ever_in_a_sealed_declared_split() {
        let fixture = eligibility_fixture().await;
        let (_, initial_role) = create_governed_cohort(
            &fixture.store,
            &fixture.snapshot,
            SnapshotSplit::Validation,
            CohortOrigin::InternalSnapshot,
            CohortRole::SealedAcceptance,
        )
        .await;
        let retired_role = CohortRoleDecision::transition(
            &initial_role,
            CohortRole::SealedAcceptance,
            CohortDisposition::Retired,
            "sealed evidence remains categorically protected",
        )
        .expect("retire role");
        GovernanceStore::append_cohort_role(&fixture.store, &retired_role)
            .await
            .expect("persist retirement");

        let protected = fixture
            .members
            .iter()
            .filter(|member| member.split == SnapshotSplit::Validation)
            .map(|member| member.source_row_id)
            .collect::<BTreeSet<_>>();
        let mut expected = fixture
            .rows
            .iter()
            .filter(|row| !protected.contains(&row.id))
            .cloned()
            .collect::<Vec<_>>();
        expected.sort_by_key(|row| row.id);
        let eligible = SOURCE_ROW_DECODES
            .scope(std::cell::Cell::new(0), async {
                let rows = fixture
                    .store
                    .list_source_rows(fixture.dataset.id)
                    .await
                    .expect("list eligible rows");
                assert_eq!(SOURCE_ROW_DECODES.with(std::cell::Cell::get), rows.len());
                rows
            })
            .await;
        assert_eq!(eligible, expected);

        let eligible_id = expected[0].id;
        let protected_id = *protected.iter().next().expect("protected member");
        let error = SOURCE_ROW_DECODES
            .scope(std::cell::Cell::new(0), async {
                let error = fixture
                    .store
                    .get_source_rows(fixture.dataset.id, vec![eligible_id, protected_id])
                    .await
                    .expect_err("an exact request containing protected evidence must fail");
                assert_eq!(SOURCE_ROW_DECODES.with(std::cell::Cell::get), 1);
                error
            })
            .await;
        assert!(error.to_string().contains("ineligible"));
        assert_eq!(
            fixture
                .store
                .get_source_rows(fixture.dataset.id, vec![eligible_id])
                .await
                .expect("ordinary exact candidate"),
            vec![expected[0].clone()]
        );
    }

    #[tokio::test]
    async fn candidate_source_withholds_development_and_external_cohort_rows() {
        let fixture = eligibility_fixture().await;
        create_governed_cohort(
            &fixture.store,
            &fixture.snapshot,
            SnapshotSplit::Validation,
            CohortOrigin::InternalSnapshot,
            CohortRole::Development,
        )
        .await;
        create_governed_cohort(
            &fixture.store,
            &fixture.snapshot,
            SnapshotSplit::Train,
            CohortOrigin::ExternalBenchmark,
            CohortRole::ExternalBenchmark,
        )
        .await;

        let development_ids = fixture
            .members
            .iter()
            .filter(|member| member.split == SnapshotSplit::Validation)
            .map(|member| member.source_row_id)
            .collect::<BTreeSet<_>>();
        let external_ids = fixture
            .members
            .iter()
            .filter(|member| member.split == SnapshotSplit::Train)
            .map(|member| member.source_row_id)
            .collect::<BTreeSet<_>>();
        let eligible = fixture
            .store
            .list_source_rows(fixture.dataset.id)
            .await
            .expect("list eligible rows");
        assert!(eligible.is_empty());
        assert!(!development_ids.is_empty());
        assert!(eligible.iter().all(|row| !external_ids.contains(&row.id)));
        let development_id = *development_ids.iter().next().expect("development member");
        assert!(
            fixture
                .store
                .get_source_rows(fixture.dataset.id, vec![development_id])
                .await
                .is_err()
        );
        let external_id = *external_ids.iter().next().expect("external member");
        assert!(
            fixture
                .store
                .get_source_rows(fixture.dataset.id, vec![external_id])
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn candidate_source_keeps_members_of_an_explicit_training_cohort() {
        let fixture = eligibility_fixture().await;
        let (_, development_role) = create_governed_cohort(
            &fixture.store,
            &fixture.snapshot,
            SnapshotSplit::Validation,
            CohortOrigin::InternalSnapshot,
            CohortRole::Development,
        )
        .await;
        let training_role = CohortRoleDecision::transition(
            &development_role,
            CohortRole::Training,
            CohortDisposition::Active,
            "explicitly release this cohort for training",
        )
        .expect("transition to training");
        GovernanceStore::append_cohort_role(&fixture.store, &training_role)
            .await
            .expect("persist training role");

        let mut expected = fixture.rows.clone();
        expected.sort_by_key(|row| row.id);
        assert_eq!(
            fixture
                .store
                .list_source_rows(fixture.dataset.id)
                .await
                .expect("training-role members remain candidates"),
            expected
        );
    }

    #[tokio::test]
    async fn candidate_source_fails_closed_when_protected_role_columns_are_tampered() {
        let fixture = eligibility_fixture().await;
        let (_, sealed_role) = create_governed_cohort(
            &fixture.store,
            &fixture.snapshot,
            SnapshotSplit::Validation,
            CohortOrigin::InternalSnapshot,
            CohortRole::SealedAcceptance,
        )
        .await;
        sqlx::query("UPDATE workflow_cohort_role_decisions SET role = 'development' WHERE id = ?")
            .bind(sealed_role.id)
            .execute(fixture.store.pool())
            .await
            .expect("tamper normalized role");

        let error = fixture
            .store
            .list_source_rows(fixture.dataset.id)
            .await
            .expect_err("tampered governance must fail closed");
        assert!(error.to_string().contains("integrity"));
    }

    #[tokio::test]
    async fn candidate_source_fails_closed_when_protected_snapshot_membership_is_deleted() {
        let fixture = eligibility_fixture().await;
        create_governed_cohort(
            &fixture.store,
            &fixture.snapshot,
            SnapshotSplit::Validation,
            CohortOrigin::InternalSnapshot,
            CohortRole::SealedAcceptance,
        )
        .await;
        let protected_id = fixture
            .members
            .iter()
            .find(|member| member.split == SnapshotSplit::Validation)
            .expect("protected member")
            .source_row_id;
        sqlx::query(
            "DELETE FROM dataset_snapshot_members WHERE snapshot_id = ? AND source_row_id = ?",
        )
        .bind(fixture.snapshot.id)
        .bind(protected_id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper snapshot membership");

        let error = SOURCE_ROW_DECODES
            .scope(std::cell::Cell::new(0), async {
                let error = fixture
                    .store
                    .list_source_rows(fixture.dataset.id)
                    .await
                    .expect_err("missing protected membership must fail closed");
                assert_eq!(SOURCE_ROW_DECODES.with(std::cell::Cell::get), 0);
                error
            })
            .await;
        assert!(error.to_string().contains("membership"));
    }

    #[tokio::test]
    async fn candidate_source_fails_closed_when_protected_member_split_is_changed() {
        let fixture = eligibility_fixture().await;
        create_governed_cohort(
            &fixture.store,
            &fixture.snapshot,
            SnapshotSplit::Validation,
            CohortOrigin::InternalSnapshot,
            CohortRole::SealedAcceptance,
        )
        .await;
        let protected_id = fixture
            .members
            .iter()
            .find(|member| member.split == SnapshotSplit::Validation)
            .expect("protected member")
            .source_row_id;
        sqlx::query(
            "UPDATE dataset_snapshot_members SET split = 'train' \
             WHERE snapshot_id = ? AND source_row_id = ?",
        )
        .bind(fixture.snapshot.id)
        .bind(protected_id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper snapshot split");

        let error = fixture
            .store
            .list_source_rows(fixture.dataset.id)
            .await
            .expect_err("changed protected split must fail closed");
        assert!(error.to_string().contains("membership"));
    }

    #[tokio::test]
    async fn attempt_commits_do_not_reload_full_run_evidence() {
        let AttemptFixture {
            store,
            plan,
            run,
            request,
            mut attempt,
            lease,
            ..
        } = attempt_fixture().await;
        let checked = CheckedAuditPlan::new(&plan).expect("checked plan");
        FULL_PLAN_LOADS
            .scope(std::cell::Cell::new(0), async {
                FULL_EVIDENCE_LOADS
                    .scope(std::cell::Cell::new(0), async {
                        store
                            .record_attempt(&checked, &lease, &attempt, &request, &run)
                            .await
                            .expect("record bounded attempt delta");
                        attempt.interrupt().expect("interrupt attempt");
                        store
                            .finish_attempt(&checked, &lease, &attempt, &[], &run)
                            .await
                            .expect("finish bounded attempt delta");
                        assert_eq!(FULL_EVIDENCE_LOADS.with(std::cell::Cell::get), 0);
                        assert_eq!(FULL_PLAN_LOADS.with(std::cell::Cell::get), 0);
                    })
                    .await;
            })
            .await;
        store
            .release_audit_execution_lease(&lease)
            .await
            .expect("release execution lease");
    }

    #[tokio::test]
    async fn audit_execution_lease_is_atomic_fenced_and_recovers_stale_owners() {
        let fixture = attempt_fixture().await;
        fixture
            .store
            .release_audit_execution_lease(&fixture.lease)
            .await
            .expect("release fixture lease");

        let (left, right) = tokio::join!(
            fixture
                .store
                .acquire_audit_execution_lease(fixture.run.id, Uuid::new_v4()),
            fixture
                .store
                .acquire_audit_execution_lease(fixture.run.id, Uuid::new_v4()),
        );
        let acquired = match (left, right) {
            (Ok(lease), Err(error)) | (Err(error), Ok(lease)) => {
                assert!(error.to_string().contains("live execution"));
                lease
            }
            outcome => panic!("exactly one same-process invocation must acquire: {outcome:?}"),
        };

        let mut wrong_token = acquired.clone();
        wrong_token.invocation_token = Uuid::new_v4();
        assert!(
            fixture
                .store
                .release_audit_execution_lease(&wrong_token)
                .await
                .is_err()
        );
        assert!(
            fixture
                .store
                .acquire_audit_execution_lease(fixture.run.id, Uuid::new_v4())
                .await
                .is_err(),
            "a wrong-token release must leave the live lease intact"
        );
        fixture
            .store
            .release_audit_execution_lease(&acquired)
            .await
            .expect("release winning lease");

        let system = System::new_all();
        let current_pid = Pid::from_u32(std::process::id());
        let eligible = |pid: Pid, process: &sysinfo::Process| {
            pid.as_u32() != 0 && pid != current_pid && process.start_time() > 0
        };
        let parent = system
            .process(current_pid)
            .and_then(sysinfo::Process::parent)
            .and_then(|pid| system.process(pid).map(|process| (pid, process)))
            .filter(|(pid, process)| eligible(*pid, process));
        let (foreign_pid, foreign_started_at) = parent
            .or_else(|| {
                system
                    .processes()
                    .iter()
                    .filter(|(pid, process)| eligible(**pid, process))
                    .min_by_key(|(_, process)| process.start_time())
                    .map(|(pid, process)| (*pid, process))
            })
            .map(|(pid, process)| (pid.as_u32(), process.start_time()))
            .expect("the test runner or operating system exposes another stable live process");
        let foreign = AuditExecutionLease::create(
            fixture.run.id,
            Uuid::new_v4(),
            foreign_pid,
            foreign_started_at,
            Utc::now(),
        )
        .expect("foreign live lease");
        let mut connection = fixture.store.pool().acquire().await.expect("connection");
        assert!(
            insert_execution_lease_in(&mut connection, &foreign)
                .await
                .expect("insert foreign lease")
        );
        drop(connection);
        assert!(
            fixture
                .store
                .acquire_audit_execution_lease(fixture.run.id, Uuid::new_v4())
                .await
                .is_err(),
            "a live foreign process owner must be retained"
        );

        sqlx::query(
            "UPDATE dataset_quality_execution_leases SET process_id = ?, \
             process_started_at = 1 WHERE run_id = ?",
        )
        .bind(i64::from(u32::MAX))
        .bind(fixture.run.id)
        .execute(fixture.store.pool())
        .await
        .expect("make owner stale");
        let recovered = fixture
            .store
            .acquire_audit_execution_lease(fixture.run.id, Uuid::new_v4())
            .await
            .expect("replace stale lease");
        assert_eq!(recovered.process_id, std::process::id());
        fixture
            .store
            .release_audit_execution_lease(&recovered)
            .await
            .expect("release recovered lease");
    }

    #[tokio::test]
    async fn audit_status_polling_does_not_hydrate_population_or_evidence() {
        let fixture = attempt_fixture().await;
        FULL_PLAN_LOADS
            .scope(std::cell::Cell::new(0), async {
                FULL_EVIDENCE_LOADS
                    .scope(std::cell::Cell::new(0), async {
                        let status = fixture
                            .store
                            .get_audit_status(fixture.run.id)
                            .await
                            .expect("checked status")
                            .expect("status exists");
                        assert_eq!(status.plan.id, fixture.plan.id);
                        assert_eq!(status.run.id, fixture.run.id);
                        assert_eq!(status.evaluator_requests, 0);
                        assert_eq!(status.evaluator_attempts, 0);
                        assert_eq!(status.assessments, 0);
                        assert_eq!(FULL_PLAN_LOADS.with(std::cell::Cell::get), 0);
                        assert_eq!(FULL_EVIDENCE_LOADS.with(std::cell::Cell::get), 0);
                    })
                    .await;
            })
            .await;

        sqlx::query(
            "UPDATE dataset_quality_audit_plans SET policy_fingerprint = 'sha256:tampered' \
             WHERE id = ?",
        )
        .bind(fixture.plan.id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper plan status fact");
        assert!(
            fixture
                .store
                .get_audit_status(fixture.run.id)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn assessment_listing_uses_one_batched_evidence_hydration() {
        let mut fixture = attempt_fixture().await;
        let checked = CheckedAuditPlan::new(&fixture.plan).expect("checked plan");
        fixture
            .store
            .record_attempt(
                &checked,
                &fixture.lease,
                &fixture.attempt,
                &fixture.request,
                &fixture.run,
            )
            .await
            .expect("record attempt");
        let assessments = successful_assessments(&fixture, &fixture.request);
        let previous_attempt = fixture.attempt.clone();
        fixture
            .attempt
            .succeed(ProviderUsage::default(), serde_json::Value::Null)
            .expect("succeed attempt");
        fixture
            .run
            .reconcile_finished_attempt(
                &fixture.plan,
                FinishedAttemptEvidence {
                    request: &fixture.request,
                    previous_attempt: &previous_attempt,
                    finished_attempt: &fixture.attempt,
                    prior_assessments: &[],
                    prior_invalid_attempts: &[],
                    new_assessments: &assessments,
                },
            )
            .expect("reconcile attempt");
        fixture
            .store
            .finish_attempt(
                &checked,
                &fixture.lease,
                &fixture.attempt,
                &assessments,
                &fixture.run,
            )
            .await
            .expect("finish attempt");

        FULL_EVIDENCE_LOADS
            .scope(std::cell::Cell::new(0), async {
                SINGLE_ASSESSMENT_HYDRATIONS
                    .scope(std::cell::Cell::new(0), async {
                        let loaded = fixture
                            .store
                            .list_assessments(fixture.run.id)
                            .await
                            .expect("list assessments");
                        assert_eq!(loaded.len(), assessments.len());
                        assert_eq!(FULL_EVIDENCE_LOADS.with(std::cell::Cell::get), 1);
                        assert_eq!(SINGLE_ASSESSMENT_HYDRATIONS.with(std::cell::Cell::get), 0);
                    })
                    .await;
            })
            .await;
    }

    #[tokio::test]
    async fn review_chains_reject_stale_branches_and_seal_after_manifest() {
        let fixture = fixture().await;
        let stale = RowQualityReview::create(
            &fixture.report,
            fixture.rows[0].id,
            None,
            RowQualityReviewDecision::Exclude,
            "second operator",
            "attempt to branch the review chain",
        )
        .expect("self-consistent stale review");
        assert!(fixture.store.append_row_review(&stale).await.is_err());

        let after_approval = CurationManifestReview::create(
            &fixture.proposal,
            Some(&fixture.approval),
            CurationManifestReviewDecision::Reject,
            "second operator",
            "attempt to alter a sealed approval",
        )
        .expect("self-consistent later review");
        assert!(
            fixture
                .store
                .append_manifest_review(&after_approval)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn new_application_rejects_a_manifest_stale_to_append_only_row_reviews() {
        let fixture = fixture().await;
        let reviews = fixture
            .store
            .list_row_reviews(fixture.report.id)
            .await
            .expect("load row reviews");
        let predecessor = reviews
            .iter()
            .rev()
            .find(|review| review.source_row_id == fixture.rows[0].id)
            .expect("row review predecessor");
        let later_review = RowQualityReview::create(
            &fixture.report,
            fixture.rows[0].id,
            Some(predecessor),
            RowQualityReviewDecision::Exclude,
            "later operator",
            "new evidence excludes this row after the old manifest was approved",
        )
        .expect("later row review");
        fixture
            .store
            .append_row_review(&later_review)
            .await
            .expect("append later row review");

        let error = fixture
            .store
            .apply_manifest(&fixture.application, &fixture.snapshot, &fixture.members)
            .await
            .expect_err("stale manifest must not authorize a new application");
        assert!(error.to_string().contains("newer row reviews"));
        let snapshot_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM dataset_snapshots WHERE id = ?")
                .bind(fixture.snapshot.id)
                .fetch_one(fixture.store.pool())
                .await
                .expect("snapshot count");
        assert_eq!(snapshot_count, 0);
    }

    #[tokio::test]
    async fn application_failure_rolls_back_the_snapshot_and_members() {
        let fixture = fixture().await;
        sqlx::query(
            "CREATE TRIGGER reject_quality_application BEFORE INSERT ON dataset_curation_applications \
             BEGIN SELECT RAISE(ABORT, 'forced application failure'); END",
        )
        .execute(fixture.store.pool())
        .await
        .expect("failure trigger");

        assert!(
            fixture
                .store
                .apply_manifest(&fixture.application, &fixture.snapshot, &fixture.members)
                .await
                .is_err()
        );
        let snapshot_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM dataset_snapshots WHERE id = ?")
                .bind(fixture.snapshot.id)
                .fetch_one(fixture.store.pool())
                .await
                .expect("snapshot count");
        let member_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM dataset_snapshot_members WHERE snapshot_id = ?",
        )
        .bind(fixture.snapshot.id)
        .fetch_one(fixture.store.pool())
        .await
        .expect("member count");
        assert_eq!((snapshot_count, member_count), (0, 0));
    }

    #[tokio::test]
    async fn attempt_reservation_and_finish_roll_back_as_atomic_units() {
        let fixture = attempt_fixture().await;
        let checked = CheckedAuditPlan::new(&fixture.plan).expect("checked plan");
        sqlx::query(
            "CREATE TRIGGER reject_quality_run_update BEFORE UPDATE ON dataset_quality_audit_runs \
             BEGIN SELECT RAISE(ABORT, 'forced run update failure'); END",
        )
        .execute(fixture.store.pool())
        .await
        .expect("reservation failure trigger");

        assert!(
            fixture
                .store
                .record_attempt(
                    &checked,
                    &fixture.lease,
                    &fixture.attempt,
                    &fixture.request,
                    &fixture.run,
                )
                .await
                .is_err()
        );
        let attempt_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM dataset_quality_evaluator_attempts")
                .fetch_one(fixture.store.pool())
                .await
                .expect("attempt count");
        assert_eq!(attempt_count, 0);
        assert_eq!(
            fixture
                .store
                .get_evaluator_request(fixture.request.id)
                .await
                .expect("request absence"),
            None
        );
        let persisted_run = fixture
            .store
            .get_audit_run(fixture.run.id)
            .await
            .expect("load unreserved run")
            .expect("run exists");
        assert_eq!(persisted_run.usage, AuditUsage::default());

        sqlx::query("DROP TRIGGER reject_quality_run_update")
            .execute(fixture.store.pool())
            .await
            .expect("drop reservation trigger");
        fixture
            .store
            .record_attempt(
                &checked,
                &fixture.lease,
                &fixture.attempt,
                &fixture.request,
                &fixture.run,
            )
            .await
            .expect("record after rollback");

        let mut terminal_attempt = fixture.attempt.clone();
        terminal_attempt
            .invalidate(
                ProviderUsage::default(),
                fixture.rows.iter().map(|row| row.id).collect(),
                "forced invalid output",
                serde_json::Value::Null,
            )
            .expect("terminal attempt");
        let mut completed_run = fixture.run.clone();
        completed_run
            .reconcile_progress(AuditProgress {
                population_rows: fixture.rows.len() as u64,
                selected_rows: fixture.rows.len() as u64,
                invalid_rows: fixture.rows.len() as u64,
                ..AuditProgress::default()
            })
            .expect("invalid progress");
        completed_run.complete(&fixture.plan).expect("complete run");

        sqlx::query(
            "CREATE TRIGGER reject_quality_run_update BEFORE UPDATE ON dataset_quality_audit_runs \
             BEGIN SELECT RAISE(ABORT, 'forced finish failure'); END",
        )
        .execute(fixture.store.pool())
        .await
        .expect("finish failure trigger");
        assert!(
            fixture
                .store
                .finish_attempt(
                    &checked,
                    &fixture.lease,
                    &terminal_attempt,
                    &[],
                    &completed_run,
                )
                .await
                .is_err()
        );
        assert_eq!(
            fixture
                .store
                .list_attempts(fixture.run.id)
                .await
                .expect("attempt after rollback"),
            vec![fixture.attempt.clone()]
        );
        assert_eq!(
            fixture
                .store
                .get_audit_run(fixture.run.id)
                .await
                .expect("run after rollback"),
            Some(fixture.run)
        );
        assert!(
            fixture
                .store
                .list_assessments(completed_run.id)
                .await
                .expect("assessments after rollback")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn independent_review_can_reclassify_assessed_progress_as_invalid() {
        let fixture = attempt_fixture().await;
        let mut before_review = fixture.run.clone();
        before_review.progress = AuditProgress {
            population_rows: 2,
            selected_rows: 2,
            assessed_rows: 1,
            borderline_rows: 1,
            pending_review_rows: 1,
            ..AuditProgress::default()
        };
        let mut after_invalid_review = before_review.clone();
        after_invalid_review.progress = AuditProgress {
            population_rows: 2,
            selected_rows: 2,
            invalid_rows: 1,
            ..AuditProgress::default()
        };

        validate_run_transition(&before_review, &after_invalid_review)
            .expect("review may replace assessed evidence with invalid output evidence");
    }

    #[tokio::test]
    async fn checked_loads_reject_normalized_and_json_tampering() {
        let fixture = fixture().await;
        sqlx::query(
            "UPDATE dataset_quality_audit_guidance \
             SET guidance_json = json_set(guidance_json, '$.unexpected', 1) WHERE plan_id = ?",
        )
        .bind(fixture.plan.id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper pinned guidance JSON");
        assert!(
            fixture
                .store
                .get_audit_guidance(fixture.plan.id)
                .await
                .is_err()
        );

        sqlx::query(
            "UPDATE dataset_curation_manifest_members SET disposition = 'exclude' \
             WHERE manifest_id = ? AND source_row_id = ?",
        )
        .bind(fixture.manifest.id)
        .bind(fixture.rows[0].id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper normalized member");
        assert!(
            fixture
                .store
                .get_manifest(fixture.manifest.id)
                .await
                .is_err()
        );

        sqlx::query(
            "UPDATE dataset_quality_reports SET report_json = json_set(report_json, '$.totals.qualified_rows', 999) \
             WHERE id = ?",
        )
        .bind(fixture.report.id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper report JSON");
        assert!(fixture.store.get_report(fixture.report.id).await.is_err());

        sqlx::query(
            "UPDATE dataset_quality_evaluator_attempts \
             SET request_json = json_set(request_json, '$.task_description', 'tampered') \
             WHERE request_id = ?",
        )
        .bind(fixture.request.id)
        .execute(fixture.store.pool())
        .await
        .expect("tamper request JSON");
        assert!(
            fixture
                .store
                .get_evaluator_request(fixture.request.id)
                .await
                .is_err()
        );
    }
}
