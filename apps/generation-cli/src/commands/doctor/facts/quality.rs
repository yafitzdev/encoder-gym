use std::collections::BTreeMap;

use artifact_core::{ArtifactKind, ProvenanceStore};
use dataset_quality_core::{
    population::NormalizedDatasetSchema,
    ports::{DatasetQualityStore, QualityCandidateSource},
};
use generation_core::ports::DatasetStore;
use uuid::Uuid;

use super::super::*;

#[derive(Debug, Default)]
struct QualityFactCounts {
    plans: usize,
    runs: usize,
    attempts: usize,
    assessments: usize,
    reports: usize,
    proposals: usize,
    manifests: usize,
    applications: usize,
}

pub(in crate::commands::doctor) async fn quality_facts_check(store: &SqliteStore) -> DoctorCheck {
    match Box::pin(verify_quality_facts(store)).await {
        Ok(counts) => pass(
            "dataset_quality_facts",
            format!(
                "verified {} plan(s), {} run(s), {} attempt(s), {} assessment(s), {} report(s), {} proposal(s), {} manifest(s), and {} qualified snapshot application(s)",
                counts.plans,
                counts.runs,
                counts.attempts,
                counts.assessments,
                counts.reports,
                counts.proposals,
                counts.manifests,
                counts.applications,
            ),
        ),
        Err(error) => fail("dataset_quality_facts", error.to_string()),
    }
}

async fn verify_quality_facts(store: &SqliteStore) -> anyhow::Result<QualityFactCounts> {
    let plans = Box::pin(verify_plans(store)).await?;
    let (runs, attempts, assessments) = Box::pin(verify_runs(store)).await?;
    let reports = Box::pin(verify_reports(store)).await?;
    let proposals = Box::pin(verify_proposals(store)).await?;
    let manifests = Box::pin(verify_provenance_roots(
        store,
        "SELECT id FROM dataset_curation_manifests ORDER BY id",
        ArtifactKind::ApprovedCurationManifest,
        "approved curation manifest",
    ))
    .await?;
    let applications = Box::pin(verify_provenance_roots(
        store,
        "SELECT id FROM dataset_curation_applications ORDER BY id",
        ArtifactKind::CurationApplication,
        "curation application",
    ))
    .await?;
    Ok(QualityFactCounts {
        plans,
        runs,
        attempts,
        assessments,
        reports,
        proposals,
        manifests,
        applications,
    })
}

async fn verify_plans(store: &SqliteStore) -> anyhow::Result<usize> {
    let mut verified = 0;
    for plan_id in ids(
        store,
        "SELECT id FROM dataset_quality_audit_plans ORDER BY id",
    )
    .await?
    {
        let plan = DatasetQualityStore::get_audit_plan(store, plan_id)
            .await?
            .with_context(|| format!("quality audit plan {plan_id} is missing"))?;
        plan.verify_integrity()?;
        let guidance = DatasetQualityStore::get_audit_guidance(store, plan_id)
            .await?
            .with_context(|| format!("quality audit plan {plan_id} guidance is missing"))?;
        guidance.verify_against(&plan)?;
        let dataset = store
            .get_dataset(plan.dataset_schema.dataset_definition_id)
            .await?
            .with_context(|| {
                format!(
                    "quality audit plan {plan_id} dataset {} is missing",
                    plan.dataset_schema.dataset_definition_id
                )
            })?;
        anyhow::ensure!(
            NormalizedDatasetSchema::from_definition(&dataset)? == plan.dataset_schema,
            "quality audit plan {plan_id} no longer matches its dataset definition"
        );
        let source_ids = plan
            .items
            .iter()
            .map(|item| item.source_row_id)
            .collect::<Vec<_>>();
        let source_rows = QualityCandidateSource::get_source_rows(
            store,
            plan.dataset_schema.dataset_definition_id,
            source_ids,
        )
        .await?;
        let by_id = source_rows
            .iter()
            .map(|row| (row.id, row))
            .collect::<BTreeMap<_, _>>();
        anyhow::ensure!(
            by_id.len() == plan.items.len()
                && plan.items.iter().all(|item| {
                    by_id.get(&item.source_row_id).is_some_and(|row| {
                        artifact_core::fingerprint(*row).ok().as_deref()
                            == Some(item.source_row_fingerprint.as_str())
                    })
                }),
            "quality audit plan {plan_id} source-set evidence differs from authoritative rows"
        );
        verified += 1;
    }
    Ok(verified)
}

async fn verify_runs(store: &SqliteStore) -> anyhow::Result<(usize, usize, usize)> {
    let mut runs = 0;
    let mut attempt_count = 0;
    let mut assessment_count = 0;
    for run_id in ids(
        store,
        "SELECT id FROM dataset_quality_audit_runs ORDER BY id",
    )
    .await?
    {
        let run = DatasetQualityStore::get_audit_run(store, run_id)
            .await?
            .with_context(|| format!("quality audit run {run_id} is missing"))?;
        let plan = DatasetQualityStore::get_audit_plan(store, run.plan_id)
            .await?
            .with_context(|| format!("quality audit run {run_id} plan is missing"))?;
        let requests = DatasetQualityStore::list_evaluator_requests(store, run_id).await?;
        let run_attempts = DatasetQualityStore::list_attempts(store, run_id).await?;
        let run_assessments = DatasetQualityStore::list_assessments(store, run_id).await?;
        run.verify_against_evidence(&plan, &requests, &run_attempts, &run_assessments)?;
        if let Some(report) = DatasetQualityStore::report_for_run(store, run_id).await? {
            report.verify_against(&plan, &run, &requests, &run_attempts, &run_assessments)?;
        }
        runs += 1;
        attempt_count += run_attempts.len();
        assessment_count += run_assessments.len();
    }
    Ok((runs, attempt_count, assessment_count))
}

async fn verify_reports(store: &SqliteStore) -> anyhow::Result<usize> {
    let mut verified = 0;
    for report_id in ids(store, "SELECT id FROM dataset_quality_reports ORDER BY id").await? {
        let report = DatasetQualityStore::get_report(store, report_id)
            .await?
            .with_context(|| format!("dataset quality report {report_id} is missing"))?;
        let run = DatasetQualityStore::get_audit_run(store, report.run_id)
            .await?
            .with_context(|| format!("dataset quality report {report_id} run is missing"))?;
        let plan = DatasetQualityStore::get_audit_plan(store, report.plan_id)
            .await?
            .with_context(|| format!("dataset quality report {report_id} plan is missing"))?;
        let requests = DatasetQualityStore::list_evaluator_requests(store, run.id).await?;
        let attempts = DatasetQualityStore::list_attempts(store, run.id).await?;
        let assessments = DatasetQualityStore::list_assessments(store, run.id).await?;
        report.verify_against(&plan, &run, &requests, &attempts, &assessments)?;
        DatasetQualityStore::list_row_reviews(store, report_id).await?;
        verified += 1;
    }
    Ok(verified)
}

async fn verify_proposals(store: &SqliteStore) -> anyhow::Result<usize> {
    let mut verified = 0;
    for proposal_id in ids(
        store,
        "SELECT id FROM dataset_curation_proposals ORDER BY id",
    )
    .await?
    {
        let proposal = DatasetQualityStore::get_curation_proposal(store, proposal_id)
            .await?
            .with_context(|| format!("curation proposal {proposal_id} is missing"))?;
        DatasetQualityStore::list_manifest_reviews(store, proposal.id).await?;
        verified += 1;
    }
    Ok(verified)
}

async fn verify_provenance_roots(
    store: &SqliteStore,
    query: &str,
    kind: ArtifactKind,
    description: &str,
) -> anyhow::Result<usize> {
    let artifact_ids = ids(store, query).await?;
    for artifact_id in &artifact_ids {
        store
            .trace_provenance(kind, *artifact_id)
            .await?
            .with_context(|| format!("{description} {artifact_id} is missing"))?;
    }
    Ok(artifact_ids.len())
}

async fn ids(store: &SqliteStore, query: &str) -> anyhow::Result<Vec<Uuid>> {
    Ok(sqlx::query_scalar::<_, Uuid>(query)
        .fetch_all(store.pool())
        .await?)
}
