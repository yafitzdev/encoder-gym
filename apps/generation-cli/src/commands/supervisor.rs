use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, ensure};
use dataset_quality_core::{
    assessment::{EvaluatorGuidance, GeneratorEvaluatorRelationship},
    policy::{
        AuditMode, BorderlineReviewPolicy, EvaluatorEgressPolicy, InvalidEvaluatorOutputPolicy,
        QualityPolicy, QualityThresholds,
    },
    population::GuidanceReference,
    ports::{QualityCandidateSource, QualityEvaluator},
};
use dataset_quality_fake::FakeQualityEvaluator;
use generation_core::{
    construction::RowConstructionPlan,
    domain::GenerationParameters,
    jobs::{GenerationBackendIdentity, GenerationExecutionPolicy, JobRunnerPolicy},
    ports::{
        BackendConfigurationStore, DatasetStore, GenerationBackend, GenerationStore,
        GenerationStrategyStore, PlanStore, RowStore,
    },
};
use generation_fake::RepairableFakeGenerationBackend;
use generation_openai_compatible::OpenAICompatibleBackend;
use generation_supervisor_core::{
    advisor::AdvisorConfiguration,
    contract::{
        AcceptedCoverageBinding, ArtifactBinding, BatchQualityThresholds, ConfigurationBinding,
        GenerationQualityContract, GeneratorIdentity, MonitoringPolicy, PromptRevisionPolicy,
        RevisionApprovalPolicy, RowQualityThresholds, SupervisorBudgets,
    },
    lifecycle::SupervisorRunState,
    ports::{GenerationSupervisorStore, SupervisorAdvisorStore},
    revision::RevisionReviewDecision,
};
use generation_supervisor_runner::orchestration::{
    GenerationQualitySupervisorRunner, RevisionReviewInput, SupervisorExecutionConfiguration,
};
use research_agent_pi_process::PiProcessRuntime;
use research_core::ports::ResearchStore;
use semantic_catalog::SemanticCatalogStore;
use serde::{Deserialize, Serialize};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::{
    cli::{
        SupervisorCommand, SupervisorDiagnoseArgs, SupervisorExecutionArgs,
        SupervisorRevisionReviewArgs, SupervisorStartArgs,
    },
    presentation,
};

const CONTRACT_FILE_SCHEMA_VERSION: u32 = 1;
const GENERATOR_PROTOCOL_VERSION: &str = "generation-supervisor-v1";
const SUPERVISOR_STACK_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SupervisorGeneratorKind {
    Fake,
    OpenaiCompatible,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SupervisorEvaluatorKind {
    Fake,
    OpenaiCompatible,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupervisorQualityPolicyFile {
    thresholds: QualityThresholds,
    invalid_output_policy: InvalidEvaluatorOutputPolicy,
    budgets: dataset_quality_core::policy::AuditBudgets,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupplementalRowRequirements {
    #[serde(default)]
    minimum_difficulty_score: Option<dataset_quality_core::policy::BasisPoints>,
    #[serde(default)]
    minimum_strategy_score: Option<dataset_quality_core::policy::BasisPoints>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupervisorContractFile {
    schema_version: u32,
    plan_id: Uuid,
    generator: SupervisorGeneratorKind,
    evaluator: SupervisorEvaluatorKind,
    quality_policy: SupervisorQualityPolicyFile,
    #[serde(default)]
    supplemental_row_requirements: SupplementalRowRequirements,
    batch_thresholds: BatchQualityThresholds,
    monitoring: MonitoringPolicy,
    budgets: SupervisorBudgets,
    approval_policy: RevisionApprovalPolicy,
    revision_policy: PromptRevisionPolicy,
}

#[derive(Debug, Serialize)]
struct SupervisorIssueView {
    events: Vec<generation_supervisor_core::lifecycle::SupervisorRunEvent>,
    windows: Vec<generation_supervisor_core::observation::BatchQualityObservation>,
    decisions: Vec<generation_supervisor_core::decision::DeterministicQualityDecision>,
}

#[derive(Debug, Serialize)]
struct RevisionView {
    session: generation_supervisor_core::advisor::AdvisorSession,
    proposal: Option<generation_supervisor_core::revision::PromptRevisionProposal>,
    review: Option<generation_supervisor_core::revision::PromptRevisionReview>,
}

#[derive(Debug, Serialize)]
struct StrategyCoverageEntry {
    scope: generation_supervisor_core::strategy::StrategyScope,
    coverage: generation_supervisor_core::strategy::StrategyCoverage,
}

pub async fn execute(command: SupervisorCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        SupervisorCommand::ContractPreview { file } => {
            presentation::print(&resolve_contract(store, &file).await?)
        }
        SupervisorCommand::ContractCreate { file } => {
            let contract = resolve_contract(store, &file).await?;
            store.create_contract(&contract).await?;
            presentation::print(&contract)
        }
        SupervisorCommand::ContractShow { id } => {
            presentation::print(&require_contract(store, id).await?)
        }
        SupervisorCommand::Start(args) => start(store, args).await,
        SupervisorCommand::Run(args) => run(store, args).await,
        SupervisorCommand::Status { id } => {
            let runner = runner_for_run(store, id, None, None).await?;
            presentation::print(&runner.status(id).await?)
        }
        SupervisorCommand::Watch { id, poll_ms } => watch(store, id, poll_ms).await,
        SupervisorCommand::Cancel { id } => {
            let runner = runner_for_run(store, id, None, None).await?;
            presentation::print(&runner.request_cancel(id).await?)
        }
        SupervisorCommand::Recover { id } => {
            let runner = runner_for_run(store, id, None, None).await?;
            presentation::print(&runner.recover(id).await?)
        }
        SupervisorCommand::Issues { id } => issues(store, id).await,
        SupervisorCommand::Diagnose(args) => diagnose(store, args).await,
        SupervisorCommand::RevisionShow { session_id } => revision_show(store, session_id).await,
        SupervisorCommand::RevisionReview(args) => revision_review(store, args).await,
        SupervisorCommand::RevisionAuthorize { id, session_id } => {
            let runner = runner_for_run(store, id, None, None).await?;
            presentation::print(&runner.authorize_revision(id, session_id, None).await?)
        }
        SupervisorCommand::Canary(args) => canary(store, args).await,
        SupervisorCommand::StrategyCoverage { id } => strategy_coverage(store, id).await,
        SupervisorCommand::TraceRow { row_id } => {
            let trace = store
                .trace_supervised_row(row_id)
                .await?
                .with_context(|| format!("supervised generated row not found: {row_id}"))?;
            presentation::print(&trace)
        }
        SupervisorCommand::Integrity => {
            let report = store.verify_supervisor_integrity().await?;
            ensure!(
                report.healthy(),
                "generation supervisor integrity check failed"
            );
            presentation::print(&report)
        }
    }
}

async fn start(store: &SqliteStore, args: SupervisorStartArgs) -> anyhow::Result<()> {
    let contract = require_contract(store, args.contract_id).await?;
    let runner = build_runner(store, &contract, None, None).await?;
    presentation::print(
        &runner
            .start(contract.id, args.guidance, args.strategy_seed)
            .await?,
    )
}

async fn run(store: &SqliteStore, args: SupervisorExecutionArgs) -> anyhow::Result<()> {
    let evaluator_env = args
        .evaluator_api_key_env
        .as_deref()
        .unwrap_or(&args.api_key_env);
    let runner =
        runner_for_run(store, args.id, Some(&args.api_key_env), Some(evaluator_env)).await?;
    let runtime = tokio::runtime::Handle::current();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("generation-supervisor".into())
        .stack_size(SUPERVISOR_STACK_BYTES)
        .spawn(move || {
            let result = runtime.block_on(runner.run_until_boundary(args.id));
            let _ = sender.send(result);
        })
        .context("could not start generation supervisor execution thread")?;
    presentation::print(
        &receiver
            .await
            .context("generation supervisor execution thread exited unexpectedly")??,
    )
}

async fn canary(store: &SqliteStore, args: SupervisorExecutionArgs) -> anyhow::Result<()> {
    let evaluator_env = args
        .evaluator_api_key_env
        .as_deref()
        .unwrap_or(&args.api_key_env);
    let runner =
        runner_for_run(store, args.id, Some(&args.api_key_env), Some(evaluator_env)).await?;
    let runtime = tokio::runtime::Handle::current();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("generation-supervisor-canary".into())
        .stack_size(SUPERVISOR_STACK_BYTES)
        .spawn(move || {
            let result = runtime.block_on(runner.run_revision_canary(args.id));
            let _ = sender.send(result);
        })
        .context("could not start generation supervisor canary thread")?;
    presentation::print(
        &receiver
            .await
            .context("generation supervisor canary thread exited unexpectedly")??,
    )
}

async fn watch(store: &SqliteStore, id: Uuid, poll_ms: u64) -> anyhow::Result<()> {
    let runner = runner_for_run(store, id, None, None).await?;
    loop {
        let status = runner.status(id).await?;
        eprintln!(
            "generation supervisor {id}: {:?}, segments={}, audits={}, observed={}, prompt=v{}",
            status.state,
            status.generation_segments,
            status.quality_audits,
            status.observed_rows,
            status.active_prompt.sequence,
        );
        if status.state.is_terminal()
            || matches!(
                status.state,
                SupervisorRunState::Paused
                    | SupervisorRunState::AwaitingReview
                    | SupervisorRunState::Canary
            )
        {
            return presentation::print(&status);
        }
        tokio::time::sleep(Duration::from_millis(poll_ms)).await;
    }
}

async fn issues(store: &SqliteStore, id: Uuid) -> anyhow::Result<()> {
    require_run(store, id).await?;
    presentation::print(&SupervisorIssueView {
        events: store.list_run_events(id).await?,
        windows: store.list_quality_windows(id).await?,
        decisions: store.list_decisions(id).await?,
    })
}

async fn diagnose(store: &SqliteStore, args: SupervisorDiagnoseArgs) -> anyhow::Result<()> {
    ensure!(
        (args.provider == "fake") == args.script.is_some(),
        "the fake Pi provider requires --script; real providers must not use a scripted turn file"
    );
    let sidecar = super::research::resolve_sidecar(&args.runtime)?;
    let mut runtime = PiProcessRuntime::new(args.runtime.node, sidecar);
    if let Some(script) = args.script {
        runtime = runtime.with_scripted_turns(super::research::read_json(&script)?);
    }
    let configuration_fingerprint = artifact_core::fingerprint(&(
        args.provider.as_str(),
        args.model.as_str(),
        args.api_key_env.as_deref(),
        "pi-jsonl-v1",
    ))?;
    let advisor = AdvisorConfiguration::create(
        args.provider,
        args.model,
        args.api_key_env,
        "pi-jsonl-v1",
        configuration_fingerprint,
    )?;
    let runner = runner_for_run(store, args.id, None, None).await?;
    let runtime_handle = tokio::runtime::Handle::current();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("generation-supervisor-diagnosis".into())
        .stack_size(SUPERVISOR_STACK_BYTES)
        .spawn(move || {
            let result =
                runtime_handle.block_on(runner.diagnose(args.id, Arc::new(runtime), advisor));
            let _ = sender.send(result);
        })
        .context("could not start generation supervisor diagnosis thread")?;
    presentation::print(
        &receiver
            .await
            .context("generation supervisor diagnosis thread exited unexpectedly")??,
    )
}

async fn revision_show(store: &SqliteStore, session_id: Uuid) -> anyhow::Result<()> {
    let session = store
        .get_session(session_id)
        .await?
        .with_context(|| format!("supervisor advisor session not found: {session_id}"))?;
    let proposal = store.latest_proposal(session_id).await?;
    let review = match &proposal {
        Some(proposal) => store.latest_revision_review(proposal.id).await?,
        None => None,
    };
    presentation::print(&RevisionView {
        session,
        proposal,
        review,
    })
}

async fn revision_review(
    store: &SqliteStore,
    args: SupervisorRevisionReviewArgs,
) -> anyhow::Result<()> {
    let decision = if args.approve {
        RevisionReviewDecision::Approve
    } else if args.reject {
        RevisionReviewDecision::Reject
    } else {
        RevisionReviewDecision::RequestRevision
    };
    let runner = runner_for_run(store, args.id, None, None).await?;
    presentation::print(
        &runner
            .authorize_revision(
                args.id,
                args.session_id,
                Some(RevisionReviewInput {
                    decision,
                    reviewer: args.reviewer,
                    rationale: args.reason,
                }),
            )
            .await?,
    )
}

async fn strategy_coverage(store: &SqliteStore, id: Uuid) -> anyhow::Result<()> {
    let run = require_run(store, id).await?;
    let contract = require_contract(store, run.contract_id).await?;
    let assignments = store
        .get_strategy_assignments(id)
        .await?
        .context("supervisor run has no strategy assignment set")?;
    let observations = store.list_row_observations(id).await?;
    let coverage = assignments
        .coverage(&contract, &observations)?
        .into_iter()
        .map(|(scope, coverage)| StrategyCoverageEntry { scope, coverage })
        .collect::<Vec<_>>();
    presentation::print(&coverage)
}

async fn resolve_contract(
    store: &SqliteStore,
    file: &PathBuf,
) -> anyhow::Result<GenerationQualityContract> {
    let input: SupervisorContractFile = super::research::read_document(file)?;
    ensure!(
        input.schema_version == CONTRACT_FILE_SCHEMA_VERSION,
        "unsupported supervisor contract file schema {}; expected {CONTRACT_FILE_SCHEMA_VERSION}",
        input.schema_version
    );
    let plan = store
        .get_plan(input.plan_id)
        .await?
        .with_context(|| format!("generation plan not found: {}", input.plan_id))?;
    let dataset = store
        .get_dataset(plan.dataset_id)
        .await?
        .with_context(|| format!("dataset not found: {}", plan.dataset_id))?;
    let evaluator: Arc<dyn QualityEvaluator> = match input.evaluator {
        SupervisorEvaluatorKind::Fake => Arc::new(FakeQualityEvaluator::default()),
        SupervisorEvaluatorKind::OpenaiCompatible => {
            super::quality::openai_compatible_primary_evaluator(store, None).await?
        }
    };
    let evaluator_identity = evaluator.identity();
    let egress_policy = match input.evaluator {
        SupervisorEvaluatorKind::Fake => EvaluatorEgressPolicy::LocalOnly,
        SupervisorEvaluatorKind::OpenaiCompatible => EvaluatorEgressPolicy::ExternalCandidateText,
    };
    let uses_external_provider =
        matches!(input.generator, SupervisorGeneratorKind::OpenaiCompatible)
            || matches!(input.evaluator, SupervisorEvaluatorKind::OpenaiCompatible);
    if uses_external_provider {
        ensure!(
            input.quality_policy.budgets.maximum_cost_microusd.is_none()
                && input.budgets.maximum_cost_microunits.is_none(),
            "OpenAI-compatible supervisor runs cannot use cost budgets until provider token pricing is explicitly pinned; token, request, row, retry, and duration budgets remain enforceable"
        );
    }
    let quality_policy = QualityPolicy::new(
        None,
        input.quality_policy.thresholds,
        input.quality_policy.invalid_output_policy,
        BorderlineReviewPolicy::None,
        input.quality_policy.budgets,
        egress_policy,
        AuditMode::FullPopulation,
    )?;
    let row_thresholds = RowQualityThresholds {
        minimum_assigned_label_score: quality_policy.thresholds.minimum_assigned_label_score,
        minimum_label_margin: quality_policy.thresholds.minimum_label_margin,
        minimum_dimension_score: quality_policy.thresholds.minimum_dimension_adherence_score,
        minimum_difficulty_score: input.supplemental_row_requirements.minimum_difficulty_score,
        minimum_authenticity_score: quality_policy.thresholds.minimum_authenticity_score,
        minimum_strategy_score: input.supplemental_row_requirements.minimum_strategy_score,
        maximum_label_leakage_risk: quality_policy.thresholds.maximum_label_leakage_risk,
        maximum_shortcut_risk: quality_policy.thresholds.maximum_shortcut_risk,
        minimum_evaluator_confidence: quality_policy.thresholds.minimum_evaluator_confidence,
    };
    let construction = RowConstructionPlan::llm_text_default()?;
    let policy = supervisor_job_policy();
    let (backend_identity, parameters) =
        resolve_generator_configuration(store, input.generator).await?;
    let generator = GeneratorIdentity::create(
        backend_identity.clone(),
        GENERATOR_PROTOCOL_VERSION,
        runtime_configuration_fingerprint(&backend_identity, &parameters, &policy, &construction)?,
    )?;
    let relationship = relationship(&backend_identity, &evaluator_identity);
    let semantic = super::semantic::resolve_dataset_semantics(store, dataset.id).await?;
    let authenticity = store.resolve_context(dataset.id).await?;
    let strategy = store.generation_strategy_context_for_plan(plan.id).await?;
    let coverage = store.dataset_cell_counts(dataset.id).await?;
    GenerationQualityContract::create(
        Uuid::new_v4(),
        ArtifactBinding::new(dataset.id, artifact_core::fingerprint(&dataset)?)?,
        ArtifactBinding::new(plan.id, artifact_core::fingerprint(&plan)?)?,
        AcceptedCoverageBinding::from_counts(&coverage)?,
        Some(ArtifactBinding::new(
            semantic.dataset_id,
            semantic.authority_fingerprint()?,
        )?),
        authenticity
            .as_ref()
            .map(|context| {
                ArtifactBinding::new(context.binding_id, context.binding_fingerprint.clone())
            })
            .transpose()?,
        ConfigurationBinding::new(construction.fingerprint.clone())?,
        strategy
            .as_ref()
            .map(|context| ArtifactBinding::new(context.id, context.fingerprint.clone()))
            .transpose()?,
        generator,
        evaluator_identity,
        relationship,
        quality_policy,
        row_thresholds,
        input.batch_thresholds,
        input.monitoring,
        input.budgets,
        input.approval_policy,
        input.revision_policy,
        chrono::Utc::now(),
    )
    .map_err(Into::into)
}

async fn runner_for_run(
    store: &SqliteStore,
    run_id: Uuid,
    generation_api_key_env: Option<&str>,
    evaluator_api_key_env: Option<&str>,
) -> anyhow::Result<GenerationQualitySupervisorRunner> {
    let run = require_run(store, run_id).await?;
    let contract = require_contract(store, run.contract_id).await?;
    build_runner(
        store,
        &contract,
        generation_api_key_env,
        evaluator_api_key_env,
    )
    .await
}

async fn build_runner(
    store: &SqliteStore,
    contract: &GenerationQualityContract,
    generation_api_key_env: Option<&str>,
    evaluator_api_key_env: Option<&str>,
) -> anyhow::Result<GenerationQualitySupervisorRunner> {
    let construction = RowConstructionPlan::llm_text_default()?;
    let policy = supervisor_job_policy();
    let (backend, identity, parameters): (
        Arc<dyn GenerationBackend>,
        GenerationBackendIdentity,
        GenerationParameters,
    ) = match contract.generator.backend.name.as_str() {
        "supervised-fake" => (
            Arc::new(RepairableFakeGenerationBackend::default()),
            GenerationBackendIdentity {
                name: "supervised-fake".into(),
                model: "repairable-deterministic-v1".into(),
                endpoint: None,
            },
            GenerationParameters::default(),
        ),
        "openai-compatible" => {
            let configuration = store
                .get_backend_configuration("openai-compatible")
                .await?
                .context("OpenAI-compatible generation backend is not configured")?;
            let base_url = configuration
                .base_url
                .as_deref()
                .context("configured generation backend has no base URL")?;
            let key = generation_api_key_env.and_then(|name| std::env::var(name).ok());
            (
                Arc::new(OpenAICompatibleBackend::new(
                    base_url,
                    key,
                    configuration.model.clone(),
                )?),
                GenerationBackendIdentity {
                    name: "openai-compatible".into(),
                    model: configuration.model.clone(),
                    endpoint: Some(base_url.trim().trim_end_matches('/').into()),
                },
                configuration.parameters,
            )
        }
        other => anyhow::bail!("unsupported supervisor generation backend: {other}"),
    };
    ensure!(
        identity == contract.generator.backend
            && runtime_configuration_fingerprint(&identity, &parameters, &policy, &construction)?
                == contract.generator.configuration_fingerprint,
        "current generation backend configuration differs from the immutable contract"
    );
    let evaluator: Arc<dyn QualityEvaluator> = match contract.evaluator.backend.as_str() {
        dataset_quality_fake::FAKE_QUALITY_BACKEND => Arc::new(FakeQualityEvaluator::default()),
        "openai-compatible" => {
            let key = evaluator_api_key_env
                .map(super::quality::read_optional_api_key)
                .transpose()?
                .flatten();
            super::quality::openai_compatible_primary_evaluator(store, key).await?
        }
        other => anyhow::bail!("unsupported supervisor quality evaluator: {other}"),
    };
    ensure!(
        evaluator.identity() == contract.evaluator,
        "configured quality evaluator differs from the immutable contract"
    );
    let semantic = super::semantic::resolve_dataset_semantics(store, contract.dataset.id).await?;
    let authenticity = store.resolve_context(contract.dataset.id).await?;
    let excerpts = super::authenticity::source_excerpts(store, authenticity.as_ref()).await?;
    let strategy = store
        .generation_strategy_context_for_plan(contract.plan.id)
        .await?;
    let mut semantic_guidance =
        super::quality::guidance_from_semantics(contract.plan.id, semantic.clone())?;
    if let Some(guidance) = &mut semantic_guidance {
        let binding = contract.semantic_context.as_ref().context(
            "resolved semantic evaluator guidance is not pinned by the supervisor contract",
        )?;
        guidance.reference = GuidanceReference::new(binding.id, binding.fingerprint.clone())?;
    }
    let authenticity_guidance = authenticity
        .clone()
        .map(super::quality::guidance_from_authenticity)
        .transpose()?
        .map(|(reference, guidance)| {
            ensure!(
                contract
                    .authenticity_context
                    .as_ref()
                    .is_some_and(|binding| {
                        binding.id == reference.id && binding.fingerprint == reference.fingerprint
                    }),
                "resolved authenticity evaluator guidance is not pinned by the supervisor contract"
            );
            Ok(guidance)
        })
        .transpose()?;
    let shared = Arc::new(store.clone());
    let generation_store: Arc<dyn GenerationStore> = shared.clone();
    let supervisor_store: Arc<dyn GenerationSupervisorStore> = shared.clone();
    let advisor_store: Arc<dyn SupervisorAdvisorStore> = shared.clone();
    let quality_store: Arc<dyn dataset_quality_core::ports::DatasetQualityStore> = shared.clone();
    let semantic_store: Arc<dyn SemanticCatalogStore> = shared.clone();
    let research_store: Arc<dyn ResearchStore> = shared.clone();
    let strategy_store: Arc<dyn GenerationStrategyStore> = shared.clone();
    let candidates: Arc<dyn QualityCandidateSource> = shared;
    GenerationQualitySupervisorRunner::new(
        generation_store,
        supervisor_store,
        advisor_store,
        quality_store,
        candidates,
        semantic_store,
        research_store,
        strategy_store,
        backend,
        vec![evaluator],
        SupervisorExecutionConfiguration {
            generation_parameters: parameters,
            generation_policy: policy,
            quality_policy: contract.quality_policy.clone(),
            evaluator_guidance: EvaluatorGuidance {
                semantic: semantic_guidance,
                authenticity: authenticity_guidance,
            },
            text_length: None,
            construction_plan: construction,
            semantic_context: Some(semantic),
            authenticity_context: authenticity,
            authenticity_source_excerpts: excerpts,
            strategy_context: strategy,
        },
    )
    .map_err(Into::into)
}

async fn resolve_generator_configuration(
    store: &SqliteStore,
    kind: SupervisorGeneratorKind,
) -> anyhow::Result<(GenerationBackendIdentity, GenerationParameters)> {
    match kind {
        SupervisorGeneratorKind::Fake => Ok((
            GenerationBackendIdentity {
                name: "supervised-fake".into(),
                model: "repairable-deterministic-v1".into(),
                endpoint: None,
            },
            GenerationParameters::default(),
        )),
        SupervisorGeneratorKind::OpenaiCompatible => {
            let configuration = store
                .get_backend_configuration("openai-compatible")
                .await?
                .context("configure the OpenAI-compatible generation backend first")?;
            let endpoint = configuration
                .base_url
                .as_deref()
                .context("configured generation backend has no base URL")?
                .trim()
                .trim_end_matches('/')
                .to_owned();
            Ok((
                GenerationBackendIdentity {
                    name: "openai-compatible".into(),
                    model: configuration.model,
                    endpoint: Some(endpoint),
                },
                configuration.parameters,
            ))
        }
    }
}

fn supervisor_job_policy() -> JobRunnerPolicy {
    JobRunnerPolicy {
        batch_size: 20,
        max_request_retries: 1,
        max_attempt_multiplier: 2,
        retry_delay: Duration::from_millis(500),
    }
}

fn runtime_configuration_fingerprint(
    identity: &GenerationBackendIdentity,
    parameters: &GenerationParameters,
    policy: &JobRunnerPolicy,
    construction: &RowConstructionPlan,
) -> anyhow::Result<String> {
    Ok(artifact_core::fingerprint(&(
        identity,
        parameters,
        GenerationExecutionPolicy::from(policy),
        construction,
    ))?)
}

fn relationship(
    generator: &GenerationBackendIdentity,
    evaluator: &dataset_quality_core::assessment::EvaluatorIdentity,
) -> GeneratorEvaluatorRelationship {
    if generator.name.eq_ignore_ascii_case(&evaluator.backend)
        && generator.model.eq_ignore_ascii_case(&evaluator.model)
    {
        GeneratorEvaluatorRelationship::SharedBackendAndModel
    } else if generator.name.eq_ignore_ascii_case(&evaluator.backend) {
        GeneratorEvaluatorRelationship::SharedBackend
    } else {
        GeneratorEvaluatorRelationship::IndependentBackend
    }
}

async fn require_contract(
    store: &SqliteStore,
    id: Uuid,
) -> anyhow::Result<GenerationQualityContract> {
    store
        .get_contract(id)
        .await?
        .with_context(|| format!("generation quality contract not found: {id}"))
}

async fn require_run(
    store: &SqliteStore,
    id: Uuid,
) -> anyhow::Result<generation_supervisor_core::lifecycle::SupervisorRun> {
    store
        .get_supervisor_run(id)
        .await?
        .with_context(|| format!("generation supervisor run not found: {id}"))
}
