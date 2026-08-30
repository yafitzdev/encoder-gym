use std::{path::Path, time::Duration};

use analysis_core::{
    domain::FindingIdentity,
    ports::{AnalysisFindingQuery, AnalysisStore, FindingEvidenceQuery},
    runner::{reproduce_report_fingerprint, verify_report_evidence},
};
use dataset_core::ports::SnapshotStore;
use evaluation_core::{
    comparison::{comparison_fingerprint, selection_fingerprint},
    domain::EvaluationRunState,
    ports::EvaluationStore,
};
use generation_core::ports::{
    BackendConfigurationStore, DatasetStore, JobStore, PlanStore, RowStore,
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
use serde::{Serialize, de::DeserializeOwned};
use synthetic_data_sqlite::SqliteStore;
use training_core::ports::{EncoderRegistry, TrainingStore};
use training_linear::verify_file;
use training_transformer::BertBundle;
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
    analysis_facts_check, evaluation_facts_check, optimization_facts_check, workflow_facts_check,
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
    checks.push(workflow_facts_check(store).await);
    checks
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
