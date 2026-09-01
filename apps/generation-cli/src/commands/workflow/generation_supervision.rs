use std::{collections::BTreeSet, sync::Arc, time::Duration};

use anyhow::{Context, ensure};
use dataset_quality_core::{
    assessment::{EvaluatorIndependence, GeneratorEvaluatorRelationship},
    ports::QualityEvaluator,
};
use dataset_quality_fake::FakeQualityEvaluator;
use dataset_quality_openai_compatible::{
    OpenAICompatibleEvaluatorConfig, OpenAICompatibleQualityEvaluator,
};
use generation_core::{
    construction::RowConstructionPlan,
    jobs::{GenerationBackendIdentity, JobRunnerPolicy},
    ports::{DatasetStore, GenerationBackend, JobStore, PlanStore, RowStore},
};
use generation_fake::RepairableFakeGenerationBackend;
use generation_openai_compatible::OpenAICompatibleBackend;
use generation_supervisor_core::{
    contract::{
        AcceptedCoverageBinding, ArtifactBinding, ConfigurationBinding, GenerationQualityContract,
        GeneratorIdentity, PreauthorizationEnvelope, RevisionApprovalPolicy,
    },
    lifecycle::{ChildKind, SupervisorRunState},
    ports::{GenerationSupervisorStore, SupervisorAdvisorStore},
    preset::{
        ResolvedEvaluatorProfile, ResolvedGenerationSupervision, ResolvedGeneratorProfile,
        ResolvedRepairApproval,
    },
};
use generation_supervisor_runner::orchestration::{
    GenerationQualitySupervisorRunner, SupervisorStatus,
};
use project_config::{ProjectConfigurationStore, ResolvedProjectConfig};
use research_core::ports::ResearchStore;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;
use workflow_core::{
    execution::WorkflowChildKind,
    ports::WorkflowRunStore,
    workflow::{
        StageAttemptState, WorkflowArtifactLink, WorkflowBudgetUsage, WorkflowDefinition,
        WorkflowRun, WorkflowStage, WorkflowStageAttempt,
    },
};

use super::{StageExecution, child_execution, link, stable_artifact_uuid};
use crate::commands::supervisor::{
    SupervisorRuntimeComponents, assemble_runner, finalize_run, relationship,
    runtime_configuration_fingerprint,
};

const GENERATOR_PROTOCOL_VERSION: &str = "generation-supervisor-v1";
const RETRY_DELAY_MILLISECONDS: u64 = 500;

pub(super) async fn execute(
    store: &SqliteStore,
    definition: &WorkflowDefinition,
    run: &WorkflowRun,
    attempt: &WorkflowStageAttempt,
    configured: &ResolvedProjectConfig,
    history: &[WorkflowStageAttempt],
    plan_id: Uuid,
) -> anyhow::Result<StageExecution> {
    let authority = definition
        .generation_supervision
        .as_ref()
        .context("supervised workflow stage has no resolved authority")?;
    authority.validate()?;
    let plan = store
        .get_plan(plan_id)
        .await?
        .with_context(|| format!("supervised workflow generation plan not found: {plan_id}"))?;
    ensure!(
        plan.dataset_id == definition.dataset_id,
        "supervised workflow plan belongs to a different dataset"
    );
    let dataset = store
        .get_dataset(plan.dataset_id)
        .await?
        .context("supervised workflow plan dataset not found")?;
    let construction = configured.row_construction_plan()?;
    let components = runtime_components(authority, construction.clone())?;
    let generator = GeneratorIdentity::create(
        components.generator_identity.clone(),
        GENERATOR_PROTOCOL_VERSION,
        runtime_configuration_fingerprint(
            &components.generator_identity,
            &components.generation_parameters,
            &components.generation_policy,
            &construction,
        )?,
    )?;
    let evaluator = components.evaluator.identity();
    let relationship = relationship(&components.generator_identity, &evaluator);
    let contract_id = stable_artifact_uuid(&serde_json::json!({
        "kind": "workflow_generation_quality_contract",
        "workflow_definition_fingerprint": definition.fingerprint,
        "workflow_run_id": run.id,
        "iteration": run.iteration,
        "stage": attempt.stage,
        "plan_id": plan.id,
        "plan_fingerprint": artifact_core::fingerprint(&plan)?,
    }))?;
    let contract = match store.get_contract(contract_id).await? {
        Some(contract) => {
            verify_contract_authority(
                &contract,
                authority,
                &dataset,
                &plan,
                &construction,
                &generator,
                &evaluator,
                relationship,
                run,
                store,
            )
            .await?;
            contract
        }
        None => {
            let contract = create_contract(
                contract_id,
                authority,
                &dataset,
                &plan,
                &construction,
                generator,
                evaluator,
                relationship,
                run,
                store,
            )
            .await?;
            store.create_contract(&contract).await?;
            contract
        }
    };
    let runner = assemble_runner(store, &contract, components).await?;
    let supervisor_run_id = stable_artifact_uuid(&serde_json::json!({
        "kind": "workflow_generation_supervisor_run",
        "contract_id": contract.id,
        "contract_fingerprint": contract.fingerprint,
        "workflow_run_id": run.id,
        "iteration": run.iteration,
        "stage": attempt.stage,
    }))?;
    let child = child_execution::reserve(
        store,
        attempt,
        WorkflowChildKind::GenerationSupervisorRun,
        "quality-supervision",
        supervisor_run_id,
    )
    .await?;
    child_execution::synchronize_parent_before_start(store, run.id, &child).await?;
    if store.get_supervisor_run(supervisor_run_id).await?.is_none() {
        runner
            .start_with_identity(
                supervisor_run_id,
                contract.id,
                Vec::new(),
                authority.generator.parameters.seed.unwrap_or(42),
            )
            .await?;
    }
    let before = runner.status(supervisor_run_id).await?;
    let status = if matches!(
        before.state,
        SupervisorRunState::Generating | SupervisorRunState::Assessing
    ) {
        runner.recover(supervisor_run_id).await?
    } else if matches!(
        before.state,
        SupervisorRunState::Queued | SupervisorRunState::Running
    ) {
        runner.run_until_boundary(supervisor_run_id).await?.status
    } else {
        before
    };
    finish_boundary(store, attempt, history, authority, &contract, status).await
}

pub(crate) async fn runner_for_governed_run(
    store: &SqliteStore,
    supervisor_run_id: Uuid,
) -> anyhow::Result<Option<GenerationQualitySupervisorRunner>> {
    let Some(parent) = store
        .get_workflow_child_execution(
            WorkflowChildKind::GenerationSupervisorRun,
            supervisor_run_id,
        )
        .await?
    else {
        return Ok(None);
    };
    let workflow_run = store
        .get_workflow_run(parent.workflow_run_id)
        .await?
        .context("governed supervisor parent workflow run not found")?;
    let definition = store
        .get_workflow_definition(workflow_run.definition_id)
        .await?
        .context("governed supervisor workflow definition not found")?;
    let authority = definition
        .generation_supervision
        .as_ref()
        .context("governed supervisor parent has no supervision authority")?;
    let configuration = store
        .get_project_configuration(definition.project_configuration_id)
        .await?
        .context("governed supervisor project configuration not found")?;
    ensure!(
        configuration.fingerprint == definition.project_configuration_fingerprint,
        "governed supervisor project configuration fingerprint changed"
    );
    let run = store
        .get_supervisor_run(supervisor_run_id)
        .await?
        .context("governed supervisor run not found")?;
    let contract = store
        .get_contract(run.contract_id)
        .await?
        .context("governed supervisor contract not found")?;
    let components =
        runtime_components(authority, configuration.resolved.row_construction_plan()?)?;
    Ok(Some(assemble_runner(store, &contract, components).await?))
}

pub(super) async fn boundary_changed(
    store: &SqliteStore,
    attempt: &WorkflowStageAttempt,
) -> anyhow::Result<bool> {
    let run_id = attempt
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "generation_supervisor_run")
        .map(|artifact| artifact.artifact_id)
        .context("supervised workflow pause has no supervisor run link")?;
    let persisted = attempt
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "generation_supervisor_boundary_event");
    let current = store.list_run_events(run_id).await?.pop();
    Ok(match (persisted, current) {
        (Some(persisted), Some(current)) => {
            persisted.artifact_id != current.id
                || persisted.artifact_fingerprint != current.fingerprint
        }
        (None, None) => false,
        _ => true,
    })
}

fn runtime_components(
    authority: &ResolvedGenerationSupervision,
    construction_plan: RowConstructionPlan,
) -> anyhow::Result<SupervisorRuntimeComponents> {
    let generator = &authority.generator;
    let generator_identity = generator_identity(generator)?;
    let backend: Arc<dyn GenerationBackend> = match generator.backend.as_str() {
        "supervised-fake" => Arc::new(RepairableFakeGenerationBackend::default()),
        "openai-compatible" => Arc::new(OpenAICompatibleBackend::new(
            generator
                .endpoint
                .as_deref()
                .context("OpenAI-compatible generator has no endpoint")?,
            read_api_key(generator.api_key_env.as_deref(), "generator")?,
            generator.model.clone(),
        )?),
        other => anyhow::bail!("unsupported governed supervisor generator: {other}"),
    };
    let evaluator: Arc<dyn QualityEvaluator> = match authority.evaluator.backend.as_str() {
        dataset_quality_fake::FAKE_QUALITY_BACKEND => Arc::new(FakeQualityEvaluator::new(
            authority.evaluator.protocol_version.clone(),
            EvaluatorIndependence::Primary,
            u64::try_from(authority.evaluator.seed)
                .context("fake evaluator seed must be non-negative")?,
        )?),
        "openai-compatible" => {
            let mut config = OpenAICompatibleEvaluatorConfig::new(
                authority
                    .evaluator
                    .endpoint
                    .as_deref()
                    .context("OpenAI-compatible evaluator has no endpoint")?,
                authority.evaluator.model.clone(),
                authority.evaluator.protocol_version.clone(),
                EvaluatorIndependence::Primary,
            );
            apply_evaluator_profile(&mut config, &authority.evaluator);
            Arc::new(OpenAICompatibleQualityEvaluator::new(
                config,
                read_api_key(authority.evaluator.api_key_env.as_deref(), "evaluator")?,
            )?)
        }
        other => anyhow::bail!("unsupported governed supervisor evaluator: {other}"),
    };
    Ok(SupervisorRuntimeComponents {
        backend,
        generator_identity,
        generation_parameters: generator.parameters.clone(),
        generation_policy: JobRunnerPolicy {
            batch_size: generator.batch_size,
            max_request_retries: generator.max_retries,
            max_attempt_multiplier: generator.max_attempt_multiplier,
            retry_delay: Duration::from_millis(RETRY_DELAY_MILLISECONDS),
        },
        construction_plan,
        evaluator,
    })
}

fn generator_identity(
    profile: &ResolvedGeneratorProfile,
) -> anyhow::Result<GenerationBackendIdentity> {
    let identity = GenerationBackendIdentity {
        name: profile.backend.clone(),
        model: profile.model.clone(),
        endpoint: profile.endpoint.clone(),
    };
    ensure!(
        !identity.name.trim().is_empty() && !identity.model.trim().is_empty(),
        "governed generator identity is incomplete"
    );
    Ok(identity)
}

fn apply_evaluator_profile(
    config: &mut OpenAICompatibleEvaluatorConfig,
    profile: &ResolvedEvaluatorProfile,
) {
    config.temperature_thousandths = profile.temperature_thousandths;
    config.seed = Some(profile.seed);
    config.timeout_millis = profile.timeout_millis;
    config.maximum_output_tokens_per_request = profile.maximum_output_tokens_per_request;
    config.maximum_response_bytes = profile.maximum_response_bytes;
    config.maximum_content_bytes = profile.maximum_content_bytes;
}

fn read_api_key(selector: Option<&str>, role: &str) -> anyhow::Result<Option<String>> {
    selector
        .map(|name| {
            std::env::var(name)
                .with_context(|| format!("{role} API-key environment variable {name} is not set"))
        })
        .transpose()
}

#[allow(clippy::too_many_arguments)]
async fn create_contract(
    id: Uuid,
    authority: &ResolvedGenerationSupervision,
    dataset: &generation_core::domain::DatasetDefinition,
    plan: &generation_core::domain::GenerationPlan,
    construction: &RowConstructionPlan,
    generator: GeneratorIdentity,
    evaluator: dataset_quality_core::assessment::EvaluatorIdentity,
    relationship: GeneratorEvaluatorRelationship,
    workflow_run: &WorkflowRun,
    store: &SqliteStore,
) -> anyhow::Result<GenerationQualityContract> {
    let semantic = crate::commands::semantic::resolve_dataset_semantics(store, dataset.id).await?;
    let authenticity = store.resolve_context(dataset.id).await?;
    let strategy = store.generation_strategy_context_for_plan(plan.id).await?;
    let coverage = store.dataset_cell_counts(dataset.id).await?;
    let minimum_scope = plan
        .cells
        .iter()
        .map(|cell| cell.target_count)
        .filter(|target| *target > 0)
        .min()
        .context("supervised generation plan has no positive targets")?;
    let mut row_thresholds = authority.row_thresholds.clone();
    if strategy.is_some() && row_thresholds.minimum_strategy_score.is_none() {
        row_thresholds.minimum_strategy_score = Some(row_thresholds.minimum_dimension_score);
    }
    GenerationQualityContract::create(
        id,
        ArtifactBinding::new(dataset.id, artifact_core::fingerprint(dataset)?)?,
        ArtifactBinding::new(plan.id, artifact_core::fingerprint(plan)?)?,
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
        evaluator,
        relationship,
        authority.quality_policy.clone(),
        row_thresholds,
        authority.batch_thresholds.clone(),
        authority.monitoring_for_minimum_scope(minimum_scope)?,
        authority.budgets.clone(),
        approval_policy(authority, workflow_run)?,
        authority.revision_policy.clone(),
        workflow_run.created_at,
    )
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
async fn verify_contract_authority(
    contract: &GenerationQualityContract,
    authority: &ResolvedGenerationSupervision,
    dataset: &generation_core::domain::DatasetDefinition,
    plan: &generation_core::domain::GenerationPlan,
    construction: &RowConstructionPlan,
    generator: &GeneratorIdentity,
    evaluator: &dataset_quality_core::assessment::EvaluatorIdentity,
    relationship: GeneratorEvaluatorRelationship,
    workflow_run: &WorkflowRun,
    store: &SqliteStore,
) -> anyhow::Result<()> {
    contract.validate()?;
    let minimum_scope = plan
        .cells
        .iter()
        .map(|cell| cell.target_count)
        .filter(|target| *target > 0)
        .min()
        .context("supervised generation plan has no positive targets")?;
    let strategy = store.generation_strategy_context_for_plan(plan.id).await?;
    let mut row_thresholds = authority.row_thresholds.clone();
    if strategy.is_some() && row_thresholds.minimum_strategy_score.is_none() {
        row_thresholds.minimum_strategy_score = Some(row_thresholds.minimum_dimension_score);
    }
    ensure!(
        contract.dataset.id == dataset.id
            && contract.dataset.fingerprint == artifact_core::fingerprint(dataset)?
            && contract.plan.id == plan.id
            && contract.plan.fingerprint == artifact_core::fingerprint(plan)?
            && contract.construction_context.fingerprint == construction.fingerprint
            && &contract.generator == generator
            && &contract.evaluator == evaluator
            && contract.generator_evaluator_relationship == relationship
            && contract.quality_policy == authority.quality_policy
            && contract.row_thresholds == row_thresholds
            && contract.batch_thresholds == authority.batch_thresholds
            && contract.monitoring == authority.monitoring_for_minimum_scope(minimum_scope)?
            && contract.budgets == authority.budgets
            && contract.approval_policy == approval_policy(authority, workflow_run)?
            && contract.revision_policy == authority.revision_policy,
        "persisted supervisor contract differs from the workflow's immutable authority"
    );
    Ok(())
}

fn approval_policy(
    authority: &ResolvedGenerationSupervision,
    workflow_run: &WorkflowRun,
) -> anyhow::Result<RevisionApprovalPolicy> {
    match authority.repair_approval {
        ResolvedRepairApproval::Manual => Ok(RevisionApprovalPolicy::ExplicitReview),
        ResolvedRepairApproval::Preauthorized {
            maximum_affected_scopes,
            valid_for_seconds,
        } => {
            let seconds = i64::try_from(valid_for_seconds)
                .context("repair preauthorization duration is too large")?;
            let expires_at = workflow_run
                .created_at
                .checked_add_signed(chrono::Duration::seconds(seconds))
                .context("repair preauthorization expiration overflowed")?;
            Ok(RevisionApprovalPolicy::FinitePreauthorization {
                envelope: PreauthorizationEnvelope {
                    revision_kind: authority.revision_policy.allowed_kind,
                    maximum_affected_scopes,
                    maximum_instructions: authority.revision_policy.maximum_instructions,
                    maximum_total_characters: authority.revision_policy.maximum_total_characters,
                    expires_at,
                },
            })
        }
    }
}

async fn finish_boundary(
    store: &SqliteStore,
    attempt: &WorkflowStageAttempt,
    history: &[WorkflowStageAttempt],
    authority: &ResolvedGenerationSupervision,
    contract: &GenerationQualityContract,
    status: SupervisorStatus,
) -> anyhow::Result<StageExecution> {
    let mut artifacts = boundary_links(store, contract, &status).await?;
    let mut usage = stage_usage(store, history, attempt, &status, authority).await?;
    match status.state {
        SupervisorRunState::Completed => {
            let outcome = finalize_run(store, status.run.id).await?;
            let iteration = attempt.stage == WorkflowStage::DatasetDiffGeneration;
            if iteration {
                usage.iterations = usage.iterations.saturating_add(1);
            }
            artifacts.extend([
                link(
                    "supervisor_qualification_handoff",
                    outcome.handoff.id,
                    &outcome.handoff.fingerprint,
                ),
                link(
                    "supervisor_qualification_application",
                    outcome.application.id,
                    &outcome.application.fingerprint,
                ),
                link(
                    if iteration {
                        "iteration_quality_audit_plan"
                    } else {
                        "quality_audit_plan"
                    },
                    outcome.handoff.replay_audit_plan.id,
                    &outcome.handoff.replay_audit_plan.fingerprint,
                ),
                link(
                    if iteration {
                        "iteration_quality_audit_run"
                    } else {
                        "quality_audit_run"
                    },
                    outcome.application.replay_audit_run.id,
                    &outcome.application.replay_audit_run.fingerprint,
                ),
                link(
                    if iteration {
                        "iteration_quality_report"
                    } else {
                        "quality_report"
                    },
                    outcome.report.id,
                    &outcome.report.fingerprint,
                ),
                link(
                    "supervisor_curation_proposal",
                    outcome.proposal.id,
                    &outcome.proposal.fingerprint,
                ),
            ]);
            Ok(StageExecution::completed(artifacts, usage))
        }
        SupervisorRunState::Paused
        | SupervisorRunState::Diagnosing
        | SupervisorRunState::AwaitingReview
        | SupervisorRunState::Canary => Ok(StageExecution {
            artifacts,
            usage,
            state: StageAttemptState::AwaitingUser,
            reason: Some(boundary_reason(store, &status).await?),
        }),
        SupervisorRunState::Failed | SupervisorRunState::Cancelled => Ok(StageExecution::failed(
            artifacts,
            usage,
            format!(
                "generation supervisor {} ended in {:?}; inspect `supervisor status {}` and `supervisor issues {}`",
                status.run.id, status.state, status.run.id, status.run.id
            ),
        )),
        state => anyhow::bail!(
            "generation supervisor {} did not reach a durable boundary: {state:?}",
            status.run.id
        ),
    }
}

async fn boundary_links(
    store: &SqliteStore,
    contract: &GenerationQualityContract,
    status: &SupervisorStatus,
) -> anyhow::Result<Vec<WorkflowArtifactLink>> {
    let mut links = vec![
        link(
            "generation_quality_contract",
            contract.id,
            &contract.fingerprint,
        ),
        link(
            "generation_supervisor_run",
            status.run.id,
            &status.run.fingerprint,
        ),
        link(
            "generation_supervisor_prompt",
            status.active_prompt.id,
            &status.active_prompt.fingerprint,
        ),
    ];
    if let Some(event) = store.list_run_events(status.run.id).await?.pop() {
        links.push(link(
            "generation_supervisor_boundary_event",
            event.id,
            &event.fingerprint,
        ));
    }
    if let Some(window) = store.list_quality_windows(status.run.id).await?.pop() {
        links.push(link(
            "generation_supervisor_quality_window",
            window.id,
            &window.fingerprint,
        ));
    }
    if let Some(decision) = store.list_decisions(status.run.id).await?.pop() {
        links.push(link(
            "generation_supervisor_decision",
            decision.id,
            &decision.fingerprint,
        ));
    }
    Ok(links)
}

async fn boundary_reason(store: &SqliteStore, status: &SupervisorStatus) -> anyhow::Result<String> {
    let id = status.run.id;
    Ok(match status.state {
        SupervisorRunState::Paused => format!(
            "generation quality evidence paused supervisor {id}; inspect `supervisor issues {id}`, then run bounded diagnosis with `supervisor diagnose {id} --script <turns.json>`"
        ),
        SupervisorRunState::Diagnosing => format!(
            "bounded diagnosis for supervisor {id} is still active; inspect `supervisor status {id}` before resuming the workflow"
        ),
        SupervisorRunState::AwaitingReview => {
            let session = store
                .list_sessions(id)
                .await?
                .pop()
                .context("awaiting-review supervisor has no advisor session")?;
            match store
                .get_contract(status.run.contract_id)
                .await?
                .context("awaiting-review supervisor contract not found")?
                .approval_policy
            {
                RevisionApprovalPolicy::ExplicitReview => format!(
                    "supervisor {id} awaits an exact prompt-repair decision; inspect `supervisor revision-show {}` and approve with `supervisor revision-review {id} {} --approve --reviewer <name> --reason <reason>`",
                    session.id, session.id
                ),
                RevisionApprovalPolicy::FinitePreauthorization { .. } => format!(
                    "supervisor {id} awaits bounded repair authorization; inspect `supervisor revision-show {}` and authorize with `supervisor revision-authorize {id} {}`",
                    session.id, session.id
                ),
            }
        }
        SupervisorRunState::Canary => format!(
            "supervisor {id} has an authorized candidate prompt; run `supervisor canary {id}` and then resume the workflow"
        ),
        _ => anyhow::bail!("supervisor is not at a user boundary"),
    })
}

async fn stage_usage(
    store: &SqliteStore,
    history: &[WorkflowStageAttempt],
    attempt: &WorkflowStageAttempt,
    status: &SupervisorStatus,
    authority: &ResolvedGenerationSupervision,
) -> anyhow::Result<WorkflowBudgetUsage> {
    let mut baseline = history
        .iter()
        .filter(|value| value.stage == attempt.stage && value.iteration == attempt.iteration)
        .min_by_key(|value| value.sequence)
        .map(|value| value.usage_after.clone())
        .unwrap_or_else(|| attempt.usage_after.clone());
    let reservations = store.list_child_reservations(status.run.id).await?;
    let mut jobs = BTreeSet::new();
    for reservation in reservations {
        if matches!(
            reservation.kind,
            ChildKind::GenerationSegment | ChildKind::RevisionCanary
        ) {
            jobs.insert(reservation.child_id);
        }
    }
    let mut generated = 0_u64;
    let mut accepted = 0_u64;
    let mut requests = 0_u64;
    for id in jobs {
        let job = store
            .get_job(id)
            .await?
            .with_context(|| format!("supervisor generation child not found: {id}"))?;
        generated = generated.saturating_add(job.generated_rows);
        accepted = accepted.saturating_add(job.accepted_rows);
        requests = requests
            .saturating_add(
                job.generated_rows
                    .div_ceil(u64::from(authority.generator.batch_size.max(1))),
            )
            .saturating_add(job.failed_requests);
    }
    baseline.accepted_rows = baseline.accepted_rows.saturating_add(accepted);
    baseline.generation_attempts = baseline.generation_attempts.saturating_add(generated);
    baseline.generation_requests = baseline.generation_requests.saturating_add(requests);
    Ok(baseline)
}
