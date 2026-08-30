use std::{path::Path, time::Duration};

use analysis_core::{
    domain::FindingIdentity,
    ports::{AnalysisFindingQuery, AnalysisStore, FindingEvidenceQuery},
    runner::{reproduce_report_fingerprint, verify_report_evidence},
};
use anyhow::Context;
use dataset_core::{
    domain::{ImportState, SnapshotSplit, SourceProvenance},
    ports::{ImportStore, SnapshotStore},
};
use evaluation_core::{
    comparison::{comparison_fingerprint, selection_fingerprint},
    domain::EvaluationRunState,
    ports::EvaluationStore,
};
use generation_core::ports::{
    BackendConfigurationStore, DatasetStore, GenerationExecutionStore, JobQuery, JobStore,
    PlanStore, RowQuery, RowStore,
};
use optimization_core::{
    campaigns::{
        CampaignArtifactKind, CampaignArtifactLink, OptimizationCampaign,
        link_candidate_evaluation, link_checkpoint, link_comparison, link_follow_up_analysis,
        link_generation_job, link_generation_plan, link_snapshot, link_training_run,
    },
    planning::verify_constrained_proposal,
    ports::{CampaignQuery, OptimizationStore, ProposalReviewQuery},
    scenarios::{OptimizationScenario, ScenarioPairComparison},
};
use project_config::{GenerationBackendKind, ResolvedProjectConfig};
use project_preparation::{BootstrapStore, PreparationStore};
use research_core::ports::ResearchStore;
use semantic_catalog::{
    GenerationSemanticAssignment, SemanticBindingDecision, SemanticCatalogStore, resolve_semantics,
};
use serde::{Serialize, de::DeserializeOwned};
use synthetic_data_sqlite::SqliteStore;
use training_core::ports::{EncoderRegistry, TrainingStore};
use training_linear::verify_file;
use training_transformer::BertBundle;
use uuid::Uuid;
use workflow_core::{
    advisor::AdvisoryAssessment,
    allocation::InitialAllocationRecord,
    approval::WorkflowApprovalDecision,
    benchmark::{AcceptanceAssessment, BenchmarkSuite},
    governance::EvidenceExposure,
    ports::{WorkflowDefinitionQuery, WorkflowRunQuery, WorkflowRunStore},
    promotion::ModelPromotion,
    stop::StopDecision,
};

use crate::{cli::DoctorArgs, presentation};

use super::config;

mod facts;

use facts::{
    analysis_facts_check, bootstrap_facts_check, evaluation_facts_check, optimization_facts_check,
    workflow_facts_check,
};

#[derive(Debug, Serialize)]
struct DoctorReport {
    healthy: bool,
    checks: Vec<DoctorCheck>,
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name: &'static str,
    status: CheckStatus,
    message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum CheckStatus {
    Pass,
    Warning,
    Fail,
    Skipped,
}

pub async fn execute(args: DoctorArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let mut checks = database_checks(store).await;
    let configured = match args.config.as_deref() {
        Some(path) => match config::resolve_path(path) {
            Ok(config) => {
                checks.push(pass(
                    "configuration",
                    format!("{} is valid ({})", path.display(), config.fingerprint()?),
                ));
                Some(config)
            }
            Err(error) => {
                checks.push(fail("configuration", error.to_string()));
                None
            }
        },
        None => {
            checks.push(skipped(
                "configuration",
                "no --config file was supplied".into(),
            ));
            None
        }
    };
    checks.push(artifact_check(configured.as_ref()));
    checks.extend(model_artifact_checks(store).await);
    checks.push(backend_check(args.check_backend, configured.as_ref(), store).await);
    let healthy = checks
        .iter()
        .all(|check| !matches!(check.status, CheckStatus::Fail));
    presentation::print(&DoctorReport { healthy, checks })?;
    anyhow::ensure!(healthy, "one or more doctor checks failed");
    Ok(())
}

async fn model_artifact_checks(store: &SqliteStore) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    match store.list_encoders().await {
        Ok(encoders) if encoders.is_empty() => checks.push(skipped(
            "registered_encoders",
            "no local encoders are registered".into(),
        )),
        Ok(encoders) => {
            let mut failures = Vec::new();
            for encoder in &encoders {
                if let Err(error) = BertBundle::verify(encoder) {
                    failures.push(format!("{} ({}): {error}", encoder.name, encoder.id));
                }
            }
            if failures.is_empty() {
                checks.push(pass(
                    "registered_encoders",
                    format!("{} registered bundle(s) verified", encoders.len()),
                ));
            } else {
                checks.push(fail("registered_encoders", failures.join("; ")));
            }
        }
        Err(error) => checks.push(fail("registered_encoders", error.to_string())),
    }

    match store.list_training_runs().await {
        Ok(runs) => {
            let mut checked = 0_usize;
            let mut failures = Vec::new();
            for run in runs {
                match store.list_checkpoints(run.id).await {
                    Ok(checkpoints) => {
                        for checkpoint in checkpoints {
                            checked += 1;
                            if let Err(error) = verify_file(
                                &checkpoint.artifact_path,
                                &checkpoint.artifact_checksum,
                                checkpoint.artifact_size_bytes,
                            ) {
                                failures.push(format!("{}: {error}", checkpoint.id));
                            }
                        }
                    }
                    Err(error) => failures.push(format!("run {}: {error}", run.id)),
                }
            }
            if !failures.is_empty() {
                checks.push(fail("training_checkpoints", failures.join("; ")));
            } else if checked == 0 {
                checks.push(skipped(
                    "training_checkpoints",
                    "no checkpoint artifacts are registered".into(),
                ));
            } else {
                checks.push(pass(
                    "training_checkpoints",
                    format!("{checked} checkpoint artifact(s) verified"),
                ));
            }
        }
        Err(error) => checks.push(fail("training_checkpoints", error.to_string())),
    }
    checks
}

async fn database_checks(store: &SqliteStore) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    match sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
        .fetch_one(store.pool())
        .await
    {
        Ok(result) if result == "ok" => checks.push(pass("database_integrity", result)),
        Ok(result) => checks.push(fail("database_integrity", result)),
        Err(error) => checks.push(fail("database_integrity", error.to_string())),
    }
    match sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(store.pool())
        .await
    {
        Ok(rows) if rows.is_empty() => checks.push(pass("foreign_keys", "no violations".into())),
        Ok(rows) => checks.push(fail("foreign_keys", format!("{} violation(s)", rows.len()))),
        Err(error) => checks.push(fail("foreign_keys", error.to_string())),
    }
    match sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM _sqlx_migrations WHERE success = 0")
        .fetch_one(store.pool())
        .await
    {
        Ok(0) => checks.push(pass(
            "migrations",
            "all applied migrations succeeded".into(),
        )),
        Ok(count) => checks.push(fail("migrations", format!("{count} failed migration(s)"))),
        Err(error) => checks.push(fail("migrations", error.to_string())),
    }
    checks.push(evaluation_facts_check(store).await);
    checks.push(analysis_facts_check(store).await);
    checks.push(optimization_facts_check(store).await);
    checks.push(bootstrap_facts_check(store).await);
    checks.push(workflow_facts_check(store).await);
    checks.push(semantic_facts_check(store).await);
    checks.push(generation_execution_facts_check(store).await);
    checks
}

async fn generation_execution_facts_check(store: &SqliteStore) -> DoctorCheck {
    let result: anyhow::Result<(usize, usize, usize)> = async {
        let jobs = store
            .list_jobs(JobQuery {
                dataset_id: None,
                plan_id: None,
                state: None,
                limit: 10_000,
                offset: 0,
            })
            .await?;
        let mut executions = 0_usize;
        let mut attempts = 0_usize;
        let mut constructed_rows = 0_usize;
        for job in jobs {
            let Some(execution) = store.get_generation_execution_spec(job.id).await? else {
                continue;
            };
            executions += 1;
            anyhow::ensure!(
                execution.reproduce_fingerprint()? == execution.fingerprint
                    && execution.job_id == job.id
                    && execution.dataset_id == job.dataset_id
                    && execution.plan_id == job.plan_id
                    && execution.backend.name == job.backend_name
                    && execution.backend.model == job.backend_model
                    && execution
                        .initial_needs
                        .iter()
                        .map(|need| u64::from(need.remaining_count))
                        .sum::<u64>()
                        == job.requested_rows,
                "generation execution {} does not reproduce its job",
                job.id
            );
            if let Some(construction_plan) = &execution.construction_plan {
                construction_plan.compile()?;
                let dataset = store
                    .get_dataset(job.dataset_id)
                    .await?
                    .with_context(|| format!("generation dataset {} is missing", job.dataset_id))?;
                construction_plan.validate_for_dataset(&dataset)?;
                let mut offset = 0_u32;
                loop {
                    let rows = store
                        .list_rows(RowQuery {
                            job_id: Some(job.id),
                            limit: 10_000,
                            offset,
                            ..RowQuery::default()
                        })
                        .await?;
                    let returned = rows.len();
                    for row in rows {
                        let trace = row.construction.as_ref().with_context(|| {
                            format!("constructed generation row {} has no field trace", row.id)
                        })?;
                        construction_plan.verify_trace(&row.text, &row.fields, trace)?;
                        constructed_rows += 1;
                    }
                    if returned < 10_000 {
                        break;
                    }
                    offset = offset
                        .checked_add(10_000)
                        .context("generation row audit offset overflowed")?;
                }
            }
            let assignment = store
                .get_generation_semantics(job.id)
                .await?
                .with_context(|| format!("generation execution {} has no semantics", job.id))?;
            anyhow::ensure!(
                assignment.context.fingerprint == execution.semantic_context_fingerprint,
                "generation execution {} semantic fingerprint differs",
                job.id
            );
            let authenticity = store.get_generation_authenticity(job.id).await?;
            anyhow::ensure!(
                execution.authenticity_context_fingerprint.as_deref()
                    == authenticity
                        .as_ref()
                        .map(|value| value.context.fingerprint.as_str()),
                "generation execution {} authenticity fingerprint differs",
                job.id
            );
            if let Some(assignment) = authenticity {
                let guard =
                    super::authenticity::source_novelty_guard(store, Some(&assignment.context))
                        .await?
                        .context("approved authenticity context has no novelty guard")?;
                anyhow::ensure!(
                    assignment.context.dataset_id == job.dataset_id
                        && assignment.context.reproduce_fingerprint()?
                            == assignment.context.fingerprint
                        && assignment.reproduce_fingerprint()? == assignment.fingerprint
                        && execution.source_novelty_guard_fingerprint.as_deref()
                            == Some(guard.fingerprint()),
                    "generation execution {} authenticity provenance is invalid",
                    job.id
                );
            }
            let job_attempts = store.list_generation_attempts(job.id).await?;
            if job_attempts.iter().any(|attempt| {
                attempt.kind
                    == generation_core::jobs::GenerationAttemptKind::DeterministicConstruction
            }) {
                let construction = execution.construction_plan.as_ref().with_context(|| {
                    format!(
                        "generation job {} has an unpinned deterministic attempt",
                        job.id
                    )
                })?;
                anyhow::ensure!(
                    construction
                        .fields
                        .iter()
                        .all(|field| !field.recipe.is_llm()),
                    "generation job {} records deterministic attempts for a plan with LLM fields",
                    job.id
                );
            }
            attempts += job_attempts.len();
            anyhow::ensure!(
                !job.state.is_terminal()
                    || job_attempts
                        .iter()
                        .all(|attempt| attempt.state.is_terminal()),
                "terminal generation job {} has an open provider request",
                job.id
            );
            let failed_requests = job_attempts
                .iter()
                .filter(|attempt| {
                    matches!(
                        attempt.state,
                        generation_core::jobs::GenerationAttemptState::Failed
                            | generation_core::jobs::GenerationAttemptState::Interrupted
                    )
                })
                .count() as u64;
            anyhow::ensure!(
                failed_requests == job.failed_requests,
                "generation job {} failed-request counter differs from persisted attempts",
                job.id
            );
            let persisted_counts = sqlx::query_as::<_, (i64, i64, i64)>(
                "SELECT COUNT(*), \
                 COALESCE(SUM(CASE WHEN validation_status = 'accepted' THEN 1 ELSE 0 END), 0), \
                 COALESCE(SUM(CASE WHEN validation_status = 'rejected' THEN 1 ELSE 0 END), 0) \
                 FROM generated_rows WHERE generation_job_id = ?",
            )
            .bind(job.id)
            .fetch_one(store.pool())
            .await?;
            anyhow::ensure!(
                u64::try_from(persisted_counts.0)? == job.generated_rows
                    && u64::try_from(persisted_counts.1)? == job.accepted_rows
                    && u64::try_from(persisted_counts.2)? == job.rejected_rows,
                "generation job {} counters differ from persisted rows",
                job.id
            );
        }
        Ok((executions, attempts, constructed_rows))
    }
    .await;
    match result {
        Ok((executions, attempts, constructed_rows)) => pass(
            "generation_execution_facts",
            format!(
                "verified {executions} execution specification(s), {attempts} durable attempt(s), and {constructed_rows} constructed row trace(s)"
            ),
        ),
        Err(error) => fail("generation_execution_facts", error.to_string()),
    }
}

async fn semantic_facts_check(store: &SqliteStore) -> DoctorCheck {
    let result: anyhow::Result<(usize, usize, usize)> = async {
        let profiles = store.list_semantic_profiles(None).await?;
        for profile in &profiles {
            anyhow::ensure!(
                profile.reproduce_fingerprint()? == profile.fingerprint,
                "semantic profile {} has a fingerprint mismatch",
                profile.id
            );
        }
        let profile_map = profiles
            .iter()
            .cloned()
            .map(|profile| (profile.id, profile))
            .collect();
        let datasets = store.list_datasets().await?;
        let mut binding_count = 0;
        for dataset in &datasets {
            let bindings = store.current_semantic_bindings(dataset.id).await?;
            binding_count += bindings.len();
            resolve_semantics(
                &super::semantic::schema_from_dataset(dataset),
                &bindings,
                &profile_map,
            )?;
        }
        let payloads = sqlx::query_scalar::<_, String>(
            "SELECT artifact_json FROM generation_job_semantics ORDER BY created_at, job_id",
        )
        .fetch_all(store.pool())
        .await?;
        for payload in &payloads {
            let assignment: GenerationSemanticAssignment = serde_json::from_str(payload)?;
            anyhow::ensure!(
                assignment.reproduce_fingerprint()? == assignment.fingerprint,
                "generation semantic assignment for job {} has a fingerprint mismatch",
                assignment.job_id
            );
            anyhow::ensure!(
                assignment.context.reproduce_fingerprint()? == assignment.context.fingerprint,
                "semantic context for job {} has a fingerprint mismatch",
                assignment.job_id
            );
            let dataset_id = sqlx::query_scalar::<_, Uuid>(
                "SELECT dataset_id FROM generation_jobs WHERE id = ?",
            )
            .bind(assignment.job_id)
            .fetch_one(store.pool())
            .await?;
            anyhow::ensure!(
                assignment.context.dataset_id == dataset_id,
                "semantic context for job {} references the wrong dataset",
                assignment.job_id
            );
            for source in &assignment.context.sources {
                let binding_payload = sqlx::query_scalar::<_, String>(
                    "SELECT artifact_json FROM dataset_semantic_binding_decisions WHERE id = ?",
                )
                .bind(source.binding_id)
                .fetch_one(store.pool())
                .await?;
                let binding: SemanticBindingDecision = serde_json::from_str(&binding_payload)?;
                anyhow::ensure!(
                    binding.fingerprint == source.binding_fingerprint
                        && binding.profile_id == Some(source.profile_id)
                        && binding.target == source.target
                        && binding.layer == source.layer,
                    "semantic source binding {} does not reproduce its job assignment",
                    source.binding_id
                );
                let profile = store
                    .get_semantic_profile(source.profile_id)
                    .await?
                    .with_context(|| {
                        format!("semantic source profile {} is missing", source.profile_id)
                    })?;
                anyhow::ensure!(
                    profile.fingerprint == source.profile_fingerprint
                        && profile.key == source.profile_key
                        && profile.version == source.profile_version,
                    "semantic source profile {} does not reproduce its job assignment",
                    source.profile_id
                );
            }
        }
        Ok((profiles.len(), binding_count, payloads.len()))
    }
    .await;
    match result {
        Ok((profiles, bindings, assignments)) => pass(
            "semantic_catalog_facts",
            format!(
                "verified {profiles} profile(s), {bindings} active binding decision(s), and {assignments} generation assignment(s)"
            ),
        ),
        Err(error) => fail("semantic_catalog_facts", error.to_string()),
    }
}

fn artifact_check(configured: Option<&ResolvedProjectConfig>) -> DoctorCheck {
    let path = configured.map_or_else(
        || Path::new("artifacts/training"),
        |config| config.training.artifact_root.as_path(),
    );
    if path.is_dir() {
        pass("artifact_directory", format!("{} exists", path.display()))
    } else if path.exists() {
        fail(
            "artifact_directory",
            format!("{} exists but is not a directory", path.display()),
        )
    } else {
        warning(
            "artifact_directory",
            format!(
                "{} will be created by the first training run",
                path.display()
            ),
        )
    }
}

async fn backend_check(
    requested: bool,
    configured: Option<&ResolvedProjectConfig>,
    store: &SqliteStore,
) -> DoctorCheck {
    if !requested {
        return skipped(
            "backend_connectivity",
            "use --check-backend to make a connectivity request".into(),
        );
    }
    if configured.is_some_and(|config| config.generation.backend == GenerationBackendKind::Fake) {
        return pass(
            "backend_connectivity",
            "the deterministic fake backend is local and available".into(),
        );
    }
    let configuration = match configured.and_then(ResolvedProjectConfig::backend_configuration) {
        Some(configuration) => Some(configuration),
        None => match store.get_backend_configuration("openai-compatible").await {
            Ok(configuration) => configuration,
            Err(error) => return fail("backend_connectivity", error.to_string()),
        },
    };
    let Some(configuration) = configuration else {
        return fail(
            "backend_connectivity",
            "the OpenAI-compatible backend is not configured".into(),
        );
    };
    let Some(base_url) = configuration.base_url else {
        return fail("backend_connectivity", "backend base URL is missing".into());
    };
    let api_key_env = configured.map_or("SYNTH_OPENAI_API_KEY", |config| {
        config.generation.api_key_env.as_str()
    });
    let endpoint = format!("{}/models", base_url.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
    {
        Ok(client) => client,
        Err(error) => return fail("backend_connectivity", error.to_string()),
    };
    let mut request = client.get(&endpoint);
    if let Ok(api_key) = std::env::var(api_key_env) {
        request = request.bearer_auth(api_key);
    }
    match request.send().await {
        Ok(response) if response.status().is_success() => pass(
            "backend_connectivity",
            format!("{} responded successfully", safe_origin(&endpoint)),
        ),
        Ok(response) => fail(
            "backend_connectivity",
            format!(
                "{} returned HTTP {}",
                safe_origin(&endpoint),
                response.status()
            ),
        ),
        Err(error) => fail(
            "backend_connectivity",
            format!("{} could not be reached: {error}", safe_origin(&endpoint)),
        ),
    }
}

fn safe_origin(url: &str) -> String {
    reqwest::Url::parse(url).map_or_else(
        |_| "configured backend".into(),
        |url| {
            let host = url.host_str().unwrap_or("configured backend");
            match url.port() {
                Some(port) => format!("{}://{host}:{port}", url.scheme()),
                None => format!("{}://{host}", url.scheme()),
            }
        },
    )
}

fn pass(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Pass,
        message,
    }
}

fn warning(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Warning,
        message,
    }
}

fn fail(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Fail,
        message,
    }
}

fn skipped(name: &'static str, message: String) -> DoctorCheck {
    DoctorCheck {
        name,
        status: CheckStatus::Skipped,
        message,
    }
}
