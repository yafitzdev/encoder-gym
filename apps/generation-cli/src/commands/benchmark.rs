use std::{collections::BTreeMap, path::Path};

use anyhow::{Context, bail, ensure};
use evaluation_core::ports::EvaluationStore;
use serde::de::DeserializeOwned;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;
use workflow_core::{
    benchmark::{
        AcceptanceState, BenchmarkCohortEvidence, BenchmarkSuite, BenchmarkSuiteDefinition,
        BenchmarkSuiteKind, CohortAssessmentInput, assess_benchmark, build_benchmark_suite,
    },
    governance::{EvidenceExposure, EvidenceExposureRequest, ExposurePurpose},
    ports::{
        AcceptanceAssessmentQuery, BenchmarkStore, BenchmarkSuiteQuery, ContaminationStore,
        GovernanceStore,
    },
};

use crate::cli::{AcceptanceStateArg, BenchmarkCommand, BenchmarkSuiteKindArg};

pub async fn execute(command: BenchmarkCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        BenchmarkCommand::Create { definition } => {
            let definition: BenchmarkSuiteDefinition = read_document(&definition)?;
            let contamination = store
                .get_contamination_report(definition.contamination_report_id)
                .await?
                .with_context(|| {
                    format!(
                        "contamination report not found: {}",
                        definition.contamination_report_id
                    )
                })?;
            let override_record = store
                .get_contamination_override(definition.contamination_report_id)
                .await?;
            let mut evidence = Vec::with_capacity(definition.request.cohorts.len());
            for requested in &definition.request.cohorts {
                let cohort = store
                    .get_cohort(requested.cohort_id)
                    .await?
                    .with_context(|| format!("cohort not found: {}", requested.cohort_id))?;
                let role = store
                    .get_current_cohort_role(requested.cohort_id)
                    .await?
                    .context("cohort has no current role")?;
                evidence.push(BenchmarkCohortEvidence { cohort, role });
            }
            let suite = build_benchmark_suite(
                definition.request,
                evidence,
                &contamination,
                override_record.as_ref(),
            )?;
            store.create_benchmark_suite(&suite).await?;
            crate::presentation::print(&suite)
        }
        BenchmarkCommand::Validate { id } => {
            let suite = require_suite(store, id).await?;
            crate::presentation::print(&validate_persisted_suite(store, &suite).await?)
        }
        BenchmarkCommand::Show { id } => {
            crate::presentation::print(&require_suite(store, id).await?)
        }
        BenchmarkCommand::List {
            kind,
            cohort_id,
            page,
        } => {
            let suites = store
                .query_benchmark_suites(BenchmarkSuiteQuery {
                    kind: kind.map(suite_kind),
                    cohort_id,
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&suites, suites.len(), page)
        }
        BenchmarkCommand::Assess {
            id,
            runs,
            comparisons,
            authorize_sealed,
        } => {
            let suite = require_suite(store, id).await?;
            let validation = validate_persisted_suite(store, &suite).await?;
            ensure!(
                validation["valid"] == true,
                "benchmark suite is no longer eligible: {validation}"
            );
            if suite.kind == BenchmarkSuiteKind::SealedAcceptance && !authorize_sealed {
                bail!("sealed assessment requires --authorize-sealed");
            }
            if suite.kind == BenchmarkSuiteKind::Development && authorize_sealed {
                bail!("--authorize-sealed is valid only for a sealed suite");
            }
            let run_ids = parse_uuid_map(&runs, "run")?;
            let comparison_ids = parse_uuid_map(&comparisons, "comparison")?;
            let mut inputs = Vec::with_capacity(suite.cohorts.len());
            for cohort in &suite.cohorts {
                let run = match run_ids.get(&cohort.cohort_id) {
                    Some(id) => Some(
                        store
                            .get_evaluation_run(*id)
                            .await?
                            .with_context(|| format!("evaluation run not found: {id}"))?,
                    ),
                    None => None,
                };
                let comparison = match comparison_ids.get(&cohort.cohort_id) {
                    Some(id) => Some(
                        store
                            .get_comparison(*id)
                            .await?
                            .with_context(|| format!("comparison not found: {id}"))?,
                    ),
                    None => None,
                };
                inputs.push(CohortAssessmentInput {
                    cohort_id: cohort.cohort_id,
                    run,
                    comparison,
                });
            }
            let assessment = assess_benchmark(&suite, inputs)?;
            let existing = store
                .query_acceptance_assessments(AcceptanceAssessmentQuery {
                    suite_id: Some(suite.id),
                    checkpoint_id: assessment.checkpoint_id,
                    state: None,
                    limit: 10_000,
                    offset: 0,
                })
                .await?
                .into_iter()
                .find(|value| {
                    value.evaluation_run_ids == assessment.evaluation_run_ids
                        && value.comparison_ids == assessment.comparison_ids
                });
            if let Some(existing) = existing {
                return crate::presentation::print(&existing);
            }
            for cohort in &suite.cohorts {
                let Some(evaluation_run_id) = assessment.evaluation_run_ids.get(&cohort.cohort_id)
                else {
                    continue;
                };
                let current_role = store
                    .get_current_cohort_role(cohort.cohort_id)
                    .await?
                    .context("cohort has no current role")?;
                let exposure = EvidenceExposure::new(
                    &store
                        .get_cohort(cohort.cohort_id)
                        .await?
                        .context("cohort not found")?,
                    &current_role,
                    EvidenceExposureRequest {
                        evaluation_run_id: Some(*evaluation_run_id),
                        workflow_run_id: None,
                        workflow_iteration: None,
                        purpose: if suite.kind == BenchmarkSuiteKind::Development {
                            ExposurePurpose::DevelopmentEvaluation
                        } else {
                            ExposurePurpose::Acceptance
                        },
                        disclosure: cohort.disclosure,
                        adaptation_eligible: cohort.adaptation_eligible,
                        note: Some(format!("benchmark suite {} assessment", suite.id)),
                    },
                )?;
                store.append_exposure(&exposure, None).await?;
            }
            store.create_acceptance_assessment(&assessment).await?;
            crate::presentation::print(&assessment)
        }
        BenchmarkCommand::AssessmentShow { id } => {
            let value = store
                .get_acceptance_assessment(id)
                .await?
                .with_context(|| format!("acceptance assessment not found: {id}"))?;
            crate::presentation::print(&value)
        }
        BenchmarkCommand::AssessmentList {
            suite_id,
            checkpoint_id,
            state,
            page,
        } => {
            let values = store
                .query_acceptance_assessments(AcceptanceAssessmentQuery {
                    suite_id,
                    checkpoint_id,
                    state: state.map(acceptance_state),
                    limit: page.limit,
                    offset: page.offset,
                })
                .await?;
            crate::presentation::print_page(&values, values.len(), page)
        }
    }
}

async fn validate_persisted_suite(
    store: &SqliteStore,
    suite: &BenchmarkSuite,
) -> anyhow::Result<serde_json::Value> {
    let mut reasons = Vec::new();
    if suite.reproduce_fingerprint()? != suite.fingerprint {
        reasons.push("suite fingerprint mismatch".to_owned());
    }
    let report = store
        .get_contamination_report(suite.contamination_report_id)
        .await?;
    if report.as_ref().map(|value| value.fingerprint.as_str())
        != Some(suite.contamination_report_fingerprint.as_str())
    {
        reasons.push("contamination report is missing or changed".into());
    }
    if let Some(expected) = &suite.contamination_override_fingerprint {
        if store
            .get_contamination_override(suite.contamination_report_id)
            .await?
            .as_ref()
            .map(|value| &value.fingerprint)
            != Some(expected)
        {
            reasons.push("contamination override is missing or changed".into());
        }
    }
    for cohort in &suite.cohorts {
        let current = store.get_current_cohort_role(cohort.cohort_id).await?;
        if current.as_ref().map(|value| value.fingerprint.as_str())
            != Some(cohort.role_decision_fingerprint.as_str())
        {
            reasons.push(format!(
                "cohort {} role changed or was retired after suite creation",
                cohort.cohort_id
            ));
        }
    }
    Ok(serde_json::json!({
        "suite_id": suite.id,
        "valid": reasons.is_empty(),
        "reasons": reasons,
    }))
}

async fn require_suite(store: &SqliteStore, id: Uuid) -> anyhow::Result<BenchmarkSuite> {
    store
        .get_benchmark_suite(id)
        .await?
        .with_context(|| format!("benchmark suite not found: {id}"))
}

fn parse_uuid_map(values: &[String], kind: &str) -> anyhow::Result<BTreeMap<Uuid, Uuid>> {
    let mut parsed = BTreeMap::new();
    for value in values {
        let (cohort, artifact) = value
            .split_once('=')
            .with_context(|| format!("{kind} mapping must be COHORT_ID=ARTIFACT_ID"))?;
        let cohort = cohort.parse().context("invalid cohort UUID")?;
        let artifact = artifact
            .parse()
            .with_context(|| format!("invalid {kind} UUID"))?;
        ensure!(
            parsed.insert(cohort, artifact).is_none(),
            "duplicate {kind} mapping for {cohort}"
        );
    }
    Ok(parsed)
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

const fn suite_kind(value: BenchmarkSuiteKindArg) -> BenchmarkSuiteKind {
    match value {
        BenchmarkSuiteKindArg::Development => BenchmarkSuiteKind::Development,
        BenchmarkSuiteKindArg::SealedAcceptance => BenchmarkSuiteKind::SealedAcceptance,
    }
}

const fn acceptance_state(value: AcceptanceStateArg) -> AcceptanceState {
    match value {
        AcceptanceStateArg::Pass => AcceptanceState::Pass,
        AcceptanceStateArg::Fail => AcceptanceState::Fail,
        AcceptanceStateArg::Inconclusive => AcceptanceState::Inconclusive,
        AcceptanceStateArg::Invalid => AcceptanceState::Invalid,
    }
}
