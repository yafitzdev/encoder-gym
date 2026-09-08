mod report;

use std::{fs, path::Path};

use anyhow::Context;
use chrono::Utc;
use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget, CampaignState, CampaignStore, ProductionCampaign,
    adopt_run_event, bind_generation_event, complete_campaign_event, first_campaign_event,
    optimization::{
        OptimizationArtifactBinding, OptimizationEventKind, OptimizationLaunchStore,
        OptimizationRunState, OptimizationView, ProductionOptimizationDefinition,
        ProductionOptimizationRun, first_optimization_event, replay_optimization,
    },
    prepare_iteration_event, record_development_event, record_sealed_authorization_event,
    replay_campaign, start_run_event,
};
use encoder_experiment_core::{
    domain::{OptimizationBudget, TrainingCandidate},
    journal::ExperimentRunState,
    ports::{EncoderTaskBackend, ExperimentStore},
    protocol::{DevelopmentSelectionRule, ExperimentProtocol},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::{ports::NativeRepairTrainingStore, proposal::RepairProposal};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use workflow_core::benchmark_generation::{BenchmarkGenerationState, BenchmarkGenerationView};

use crate::{
    cli::{
        EncoderOptimizeAuthorizeArgs, EncoderOptimizeCancelArgs, EncoderOptimizeCommand,
        EncoderOptimizeManifestArgs, EncoderOptimizeRunArgs, NomosWorkspaceArgs,
    },
    commands::{
        experiment::{ensure_database_belongs_to_workspace, load_persisted_view},
        production_campaign::{
            CampaignContext, campaign_provenance, finalize_campaign_iteration, load_generation,
        },
        production_repair::{load_approved_delta_context, load_approved_delta_facts},
    },
    presentation,
};

const MANIFEST_SCHEMA_VERSION: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OptimizeAdapter {
    Nomos,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ApprovalMode {
    ExplicitSealedUse,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FinalDecisionPolicy {
    StrictMetricContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExistingExperimentManifest {
    protocol_id: Uuid,
    protocol_fingerprint: String,
    run_id: Uuid,
    run_head_fingerprint: String,
}

/// This intentionally begins after both human review boundaries. Earlier repair commands own
/// proposal and delta review; optimize consumes their immutable approved training snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptimizeManifest {
    schema_version: u32,
    name: String,
    adapter: OptimizeAdapter,
    project_source_revision: String,
    training_snapshot_id: Uuid,
    training_snapshot_fingerprint: String,
    training_snapshot_specification_fingerprint: String,
    benchmark_generation_id: Uuid,
    benchmark_generation_fingerprint: String,
    approval_mode: ApprovalMode,
    selection_policy: DevelopmentSelectionRule,
    final_decision_policy: FinalDecisionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    existing_experiment: Option<ExistingExperimentManifest>,
}

struct ResolvedLaunch {
    manifest: OptimizeManifest,
    manifest_fingerprint: String,
    definition: ProductionOptimizationDefinition,
    adopted_protocol_id: Option<Uuid>,
    adopted_run_id: Option<Uuid>,
}

struct LaunchContext {
    definition: ProductionOptimizationDefinition,
    run: ProductionOptimizationRun,
    view: OptimizationView,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedOptimizationReadiness {
    pub manifest_fingerprint: String,
    pub specification_fingerprint: String,
    pub project_id: Uuid,
    pub project_fingerprint: String,
    pub training_snapshot_id: Uuid,
    pub benchmark_generation_id: Uuid,
    pub candidate_count: usize,
    pub budget: CampaignBudget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_run: Option<ManagedExistingOptimization>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedExistingOptimization {
    pub run_id: Uuid,
    pub state: OptimizationRunState,
}

/// Resolve the same immutable definition consumed by `start`, without writing
/// state or opening a native adapter. Managed-project readiness uses this
/// instead of reproducing optimization policy in the presentation layer.
pub(crate) async fn managed_readiness(
    store: &SqliteExperimentStore,
    manifest: &Path,
) -> anyhow::Result<ManagedOptimizationReadiness> {
    let resolved = resolve_manifest(store, manifest).await?;
    let existing_run = match store
        .find_optimization_by_manifest(resolved.manifest_fingerprint.clone())
        .await?
    {
        Some((definition, run)) => {
            if definition.specification_fingerprint != resolved.definition.specification_fingerprint
            {
                anyhow::bail!(
                    "manifest identity already belongs to a different resolved optimization"
                );
            }
            let context = load_launch(store, run.id).await?;
            Some(ManagedExistingOptimization {
                run_id: run.id,
                state: context.view.state,
            })
        }
        None => None,
    };
    Ok(ManagedOptimizationReadiness {
        manifest_fingerprint: resolved.manifest_fingerprint,
        specification_fingerprint: resolved.definition.specification_fingerprint,
        project_id: resolved.definition.project.id,
        project_fingerprint: resolved.definition.project.fingerprint,
        training_snapshot_id: resolved.definition.training_snapshot.id,
        benchmark_generation_id: resolved.definition.benchmark.generation_id,
        candidate_count: resolved.definition.candidates.len(),
        budget: resolved.definition.campaign_budget,
        existing_run,
    })
}

pub async fn execute(command: EncoderOptimizeCommand, database_url: &str) -> anyhow::Result<()> {
    let args = backend_args(&command);
    ensure_database_belongs_to_workspace(database_url, &args.workspace)?;
    let workspace = args.workspace.clone();
    let python = args.python.clone();
    let store = command.database_access().production(database_url).await?;
    match command {
        EncoderOptimizeCommand::Doctor(args) => {
            let backend = NomosBackend::open(&workspace, python)?;
            doctor(&store, &backend, args).await
        }
        command => {
            execute_lifecycle(command, &store, || {
                NomosBackend::open(&workspace, python).map_err(Into::into)
            })
            .await
        }
    }
}

/// The compiled composition root supplies a task adapter only for native stages.
/// Passive commands and launch validation consume the same persisted slice contracts.
pub(crate) async fn execute_lifecycle<B: EncoderTaskBackend>(
    command: EncoderOptimizeCommand,
    store: &SqliteExperimentStore,
    backend_factory: impl FnOnce() -> anyhow::Result<B>,
) -> anyhow::Result<()> {
    match command {
        EncoderOptimizeCommand::Preview(args) => preview(store, args).await,
        EncoderOptimizeCommand::Start(args) => start(store, args).await,
        EncoderOptimizeCommand::Status(args) => status(store, args).await,
        EncoderOptimizeCommand::Inspect(args) => inspect(store, args).await,
        EncoderOptimizeCommand::ReviewRepair(args) => review_facts(store, args, false).await,
        EncoderOptimizeCommand::ReviewDelta(args) => review_facts(store, args, true).await,
        EncoderOptimizeCommand::Resume(args) => resume(store, backend_factory, args).await,
        EncoderOptimizeCommand::AuthorizeExternal(args) => authorize_external(store, args).await,
        EncoderOptimizeCommand::AuthorizeSealed(args) => {
            let backend = backend_factory()?;
            authorize_sealed(store, &backend, args).await
        }
        EncoderOptimizeCommand::Cancel(args) => cancel(store, args).await,
        EncoderOptimizeCommand::Provenance(args) => {
            let backend = backend_factory()?;
            provenance(store, &backend, args).await
        }
        EncoderOptimizeCommand::Report(args) => report::execute(store, args).await,
        EncoderOptimizeCommand::Doctor(_) => {
            anyhow::bail!("native Doctor is handled by the adapter composition root")
        }
    }
}

async fn preview(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeManifestArgs,
) -> anyhow::Result<()> {
    let resolved = resolve_manifest(store, &args.manifest).await?;
    presentation::print(&serde_json::json!({
        "persisted": false,
        "external_calls": 0,
        "manifest_fingerprint": resolved.manifest_fingerprint,
        "specification_fingerprint": resolved.definition.specification_fingerprint,
        "project": resolved.definition.project,
        "training_snapshot": resolved.definition.training_snapshot,
        "benchmark": resolved.definition.benchmark,
        "candidate_count": resolved.definition.candidates.len(),
        "budget": resolved.definition.campaign_budget,
        "review_boundaries": {
            "repair_proposal": "already_frozen",
            "native_delta": "already_frozen",
            "sealed_use": "explicit_runtime_authorization",
        },
        "adopts_existing_experiment": resolved.manifest.existing_experiment.is_some(),
        "stages": [
            "attach_campaign",
            "prepare_protocol",
            "start_or_adopt_run",
            "train_and_evaluate_development",
            "finalize_or_await_sealed_authorization",
            "complete",
        ],
    }))
}

async fn start(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeManifestArgs,
) -> anyhow::Result<()> {
    let manifest = read_manifest(&args.manifest)?;
    validate_manifest(&manifest)?;
    let manifest_fingerprint = artifact_core::fingerprint(&manifest)?;
    if let Some((_, run)) = store
        .find_optimization_by_manifest(manifest_fingerprint)
        .await?
    {
        let context = load_launch(store, run.id).await?;
        return print_current_status(store, &context, true).await;
    }
    let resolved = resolve_manifest(store, &args.manifest).await?;
    if let Some((definition, run)) = store
        .find_optimization_by_manifest(resolved.manifest_fingerprint.clone())
        .await?
    {
        if definition.specification_fingerprint != resolved.definition.specification_fingerprint {
            anyhow::bail!("manifest identity already belongs to a different resolved optimization");
        }
        let context = load_launch(store, run.id).await?;
        return print_current_status(store, &context, true).await;
    }
    let now = Utc::now();
    let run = ProductionOptimizationRun::create_with_reservations(
        &resolved.definition,
        Uuid::new_v4(),
        resolved.adopted_protocol_id.unwrap_or_else(Uuid::new_v4),
        resolved.adopted_run_id.unwrap_or_else(Uuid::new_v4),
        now,
    )?;
    let first = first_optimization_event(&run, now)?;
    store
        .create_optimization(resolved.definition, run.clone(), first)
        .await?;
    let context = load_launch(store, run.id).await?;
    print_current_status(store, &context, false).await
}

async fn status(store: &SqliteExperimentStore, args: EncoderOptimizeRunArgs) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    print_current_status(store, &context, true).await
}

async fn inspect(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let campaign = load_optional_campaign(store, &context).await?;
    presentation::print(&serde_json::json!({
        "definition": context.definition,
        "run": context.run,
        "lifecycle": context.view,
        "campaign": campaign.as_ref().map(|value| &value.view),
        "experiment": campaign.as_ref().and_then(|value| value.experiment.as_ref()),
        "next_command": next_command(&context, campaign.as_ref()),
    }))
}

async fn review_facts(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeRunArgs,
    delta: bool,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let snapshot = store
        .get_native_repair_training_snapshot(context.definition.training_snapshot.id)
        .await?
        .context("optimization training snapshot does not exist")?;
    let approved = load_approved_delta_facts(store, snapshot.selection.id).await?;
    let value = if delta {
        serde_json::json!({
            "boundary": "native_delta_review",
            "status": "already_frozen",
            "selection": approved.selection,
            "approval": approved.approval,
            "predecessor": approved.approval_predecessor,
            "note": "Use production-repair before optimize to create a different review; this run cannot rewrite frozen authority.",
        })
    } else {
        serde_json::json!({
            "boundary": "repair_proposal_review",
            "status": "already_frozen",
            "proposal": {
                "id": approved.proposal.id,
                "fingerprint": approved.proposal.fingerprint,
                "specification_fingerprint": approved.proposal.specification_fingerprint,
            },
            "application": approved.selection.application,
            "note": "Use production-repair before optimize to create a different review; this run cannot rewrite frozen authority.",
        })
    };
    presentation::print(&value)
}

async fn resume<B: EncoderTaskBackend>(
    store: &SqliteExperimentStore,
    backend_factory: impl FnOnce() -> anyhow::Result<B>,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    if context.view.state != OptimizationRunState::Planned
        && context.view.state != OptimizationRunState::CampaignActive
    {
        return print_current_status(store, &context, true).await;
    }
    if context.view.state == OptimizationRunState::Planned {
        attach_campaign(store, &context).await?;
        return status(
            store,
            EncoderOptimizeRunArgs {
                run_id: args.run_id,
                backend: args.backend,
            },
        )
        .await;
    }

    let campaign = load_optimization_campaign(store, context.run.reserved_campaign_id).await?;
    match campaign.view.state {
        CampaignState::AwaitingGeneration => {
            ensure_generation_current(store, &context.definition).await?;
            let event = bind_generation_event(
                &campaign.campaign,
                &campaign.view,
                context.definition.benchmark.clone(),
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::ReadyToPrepare => {
            ensure_generation_current(store, &context.definition).await?;
            let backend = backend_factory()?;
            let runner = ExperimentRunner::new(store, &backend);
            let protocol = runner
                .prepare_multi_protocol_identified(
                    context.run.reserved_protocol_id,
                    context.definition.project.id,
                    context.definition.metric_contract.clone(),
                    protocol_budget(&context.definition)?,
                    context.definition.maximum_evaluation_seconds,
                    context.definition.development_suite_keys.clone(),
                    context.definition.sealed_suite_key.clone(),
                    context.definition.candidates.clone(),
                )
                .await?;
            let event =
                prepare_iteration_event(&campaign.campaign, &campaign.view, &protocol, Utc::now())?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::ReadyToStart => {
            ensure_generation_current(store, &context.definition).await?;
            let backend = backend_factory()?;
            let runner = ExperimentRunner::new(store, &backend);
            let experiment = runner
                .create_run_identified(
                    context.run.reserved_protocol_id,
                    context.run.reserved_experiment_run_id,
                )
                .await?;
            let event = if experiment.state == ExperimentRunState::Ready {
                start_run_event(&campaign.campaign, &campaign.view, &experiment, Utc::now())?
            } else {
                adopt_run_event(&campaign.campaign, &campaign.view, &experiment, Utc::now())?
            };
            store.append_campaign_event(&event).await?;
        }
        CampaignState::RunningDevelopment => {
            ensure_generation_current(store, &context.definition).await?;
            let current = campaign
                .experiment
                .context("campaign experiment is missing")?;
            let experiment = if matches!(
                current.state,
                ExperimentRunState::AwaitingSealedAuthorization | ExperimentRunState::Completed
            ) {
                current
            } else {
                let backend = backend_factory()?;
                ExperimentRunner::new(store, &backend)
                    .run_development(context.run.reserved_experiment_run_id)
                    .await?
            };
            let event = record_development_event(
                &campaign.campaign,
                &campaign.view,
                &experiment,
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::AwaitingFinalization => {
            let experiment = campaign
                .experiment
                .as_ref()
                .context("campaign experiment is missing")?;
            finalize_campaign_iteration(store, &campaign, experiment).await?;
        }
        CampaignState::AwaitingSealedAuthorization => {
            return print_status(&context, true, Some(&campaign));
        }
        CampaignState::SealedAuthorized => {
            ensure_generation_current(store, &context.definition).await?;
            let experiment = match campaign.experiment.as_ref() {
                Some(value) if value.state == ExperimentRunState::Completed => value.clone(),
                _ => {
                    let backend = backend_factory()?;
                    ExperimentRunner::new(store, &backend)
                        .run_sealed(context.run.reserved_experiment_run_id)
                        .await?
                }
            };
            finalize_campaign_iteration(store, &campaign, &experiment).await?;
        }
        CampaignState::RenewalRequired => {
            let event = complete_campaign_event(
                &campaign.campaign,
                &campaign.view,
                "finite optimize run completed its one authorized iteration",
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
        }
        CampaignState::Completed => {
            let decision = campaign
                .experiment
                .as_ref()
                .and_then(|value| value.final_decision)
                .context("completed campaign has no deterministic experiment decision")?;
            let event = context.view.next_event(
                &context.run,
                OptimizationEventKind::Completed {
                    decision,
                    campaign_head_fingerprint: campaign.view.last_event_fingerprint,
                },
                Utc::now(),
            )?;
            store.append_optimization_event(event).await?;
        }
    }
    status(
        store,
        EncoderOptimizeRunArgs {
            run_id: args.run_id,
            backend: args.backend,
        },
    )
    .await
}

async fn authorize_sealed(
    store: &SqliteExperimentStore,
    backend: &impl EncoderTaskBackend,
    args: EncoderOptimizeAuthorizeArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    if context.view.state != OptimizationRunState::CampaignActive {
        anyhow::bail!("optimization is not active");
    }
    let campaign = load_optimization_campaign(store, context.run.reserved_campaign_id).await?;
    if !matches!(
        campaign.view.state,
        CampaignState::AwaitingSealedAuthorization | CampaignState::SealedAuthorized
    ) {
        anyhow::bail!("optimization is not waiting for sealed-use authorization");
    }
    ensure_generation_current(store, &context.definition).await?;
    let runner = ExperimentRunner::new(store, backend);
    let experiment = runner
        .authorize_sealed(context.run.reserved_experiment_run_id, &args.authorized_by)
        .await?;
    if campaign.view.state == CampaignState::SealedAuthorized {
        return print_current_status(store, &context, true).await;
    }
    let event = record_sealed_authorization_event(
        &campaign.campaign,
        &campaign.view,
        &experiment,
        args.authorized_by,
        Utc::now(),
    )?;
    store.append_campaign_event(&event).await?;
    let updated = load_launch(store, args.run_id).await?;
    print_current_status(store, &updated, true).await
}

async fn authorize_external(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeAuthorizeArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let snapshot = store
        .get_native_repair_training_snapshot(context.definition.training_snapshot.id)
        .await?
        .context("optimization training snapshot does not exist")?;
    let approved = load_approved_delta_facts(store, snapshot.selection.id).await?;
    if approved.proposal.budget.maximum_external_calls == 0 {
        return presentation::print(&serde_json::json!({
            "run_id": context.run.id,
            "status": "not_required",
            "authorized": false,
            "authorized_by": args.authorized_by,
            "maximum_external_calls": 0,
            "reason": "The frozen repair proposal authorizes no network or paid provider calls.",
            "next_command": next_command(&context, None),
        }));
    }
    anyhow::bail!(
        "this optimize run has no persisted external-call reservation; create and authorize the exact request through production-repair before starting optimize"
    )
}

async fn cancel(
    store: &SqliteExperimentStore,
    args: EncoderOptimizeCancelArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    if !matches!(
        context.view.state,
        OptimizationRunState::Planned | OptimizationRunState::CampaignActive
    ) {
        return print_current_status(store, &context, true).await;
    }
    let event = context.view.next_event(
        &context.run,
        OptimizationEventKind::Cancelled {
            reason: args.reason,
        },
        Utc::now(),
    )?;
    store.append_optimization_event(event).await?;
    let updated = load_launch(store, args.run_id).await?;
    print_current_status(store, &updated, true).await
}

async fn doctor(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let snapshot = store
        .get_native_repair_training_snapshot(context.definition.training_snapshot.id)
        .await?
        .context("optimization training snapshot does not exist")?;
    let approved = load_approved_delta_context(store, backend, snapshot.selection.id, true).await?;
    snapshot.validate_against(
        &approved.project,
        &approved.proposal,
        &approved.candidate_set,
        &approved.report,
        &approved.approval,
        approved.approval_predecessor.as_ref(),
        &approved.selection,
    )?;
    backend.inspect(approved.project.clone()).await?;
    ensure_generation_matches(store, &context.definition, false).await?;
    let campaign_fingerprint = if store
        .get_campaign(context.run.reserved_campaign_id)
        .await?
        .is_some()
    {
        Some(artifact_core::fingerprint(
            &campaign_provenance(store, backend, context.run.reserved_campaign_id).await?,
        )?)
    } else {
        None
    };
    presentation::print(&serde_json::json!({
        "verified": true,
        "run_id": context.run.id,
        "state": context.view.state,
        "optimization_head_fingerprint": context.view.last_event_fingerprint,
        "campaign_provenance_fingerprint": campaign_fingerprint,
        "native_replay_verified": true,
    }))
}

async fn provenance(
    store: &SqliteExperimentStore,
    backend: &impl EncoderTaskBackend,
    args: EncoderOptimizeRunArgs,
) -> anyhow::Result<()> {
    let context = load_launch(store, args.run_id).await?;
    let campaign = if store
        .get_campaign(context.run.reserved_campaign_id)
        .await?
        .is_some()
    {
        Some(campaign_provenance(store, backend, context.run.reserved_campaign_id).await?)
    } else {
        None
    };
    let bundle = serde_json::json!({
        "schema_version": 1,
        "definition": context.definition,
        "run": context.run,
        "lifecycle": context.view,
        "campaign": campaign,
    });
    presentation::print(&serde_json::json!({
        "bundle": bundle,
        "bundle_fingerprint": artifact_core::fingerprint(&bundle)?,
    }))
}

async fn resolve_manifest(
    store: &SqliteExperimentStore,
    path: &Path,
) -> anyhow::Result<ResolvedLaunch> {
    let manifest = read_manifest(path)?;
    validate_manifest(&manifest)?;
    let manifest_fingerprint = artifact_core::fingerprint(&manifest)?;
    let snapshot = store
        .get_native_repair_training_snapshot(manifest.training_snapshot_id)
        .await?
        .with_context(|| {
            format!(
                "native repair training snapshot {} does not exist",
                manifest.training_snapshot_id
            )
        })?;
    if snapshot.fingerprint != manifest.training_snapshot_fingerprint
        || snapshot.specification_fingerprint
            != manifest.training_snapshot_specification_fingerprint
    {
        anyhow::bail!("optimization manifest training snapshot binding changed");
    }
    let approved = load_approved_delta_facts(store, snapshot.selection.id).await?;
    snapshot.validate_against(
        &approved.project,
        &approved.proposal,
        &approved.candidate_set,
        &approved.report,
        &approved.approval,
        approved.approval_predecessor.as_ref(),
        &approved.selection,
    )?;
    if approved.project.source_revision != manifest.project_source_revision {
        anyhow::bail!("optimization manifest project revision differs from the immutable project");
    }

    let source_view =
        load_persisted_view(approved.proposal.context.source_experiment_run_id, store).await?;
    let source_protocol = store
        .get_protocol(source_view.protocol_id)
        .await?
        .context("repair metric-source protocol does not exist")?;
    let source_project = store
        .get_project(source_protocol.project_snapshot_id)
        .await?
        .context("repair metric-source project does not exist")?;
    source_protocol.validate_integrity(&source_project)?;

    let (generation, generation_view) =
        load_generation(store, manifest.benchmark_generation_id).await?;
    if generation.fingerprint != manifest.benchmark_generation_fingerprint {
        anyhow::bail!("optimization manifest benchmark generation binding changed");
    }
    require_active_generation(&generation_view, &generation)?;
    let benchmark = CampaignBenchmarkBinding::from_active_generation(
        &generation,
        &generation_view,
        source_protocol.sealed_suite_key.clone(),
    )?;
    let repair_benchmark = &approved.proposal.context.benchmark;
    if repair_benchmark.generation_id != generation.id
        || repair_benchmark.generation_fingerprint != generation.fingerprint
        || repair_benchmark.journal_sequence != generation_view.last_sequence
        || repair_benchmark.journal_head_fingerprint != generation_view.last_event_fingerprint
        || repair_benchmark.development_suite_fingerprints
            != benchmark.development_suite_fingerprints
        || repair_benchmark.sealed_suite_id != benchmark.sealed_suite_id
        || repair_benchmark.sealed_suite_fingerprint != benchmark.sealed_suite_fingerprint
    {
        anyhow::bail!("approved repair and optimization benchmark authorities differ");
    }

    let compiled = snapshot.compile_training_candidates(&approved.project, &approved.proposal)?;
    let deterministic = compiled
        .into_iter()
        .map(|candidate| {
            TrainingCandidate::create_identified(
                deterministic_uuid(
                    &manifest_fingerprint,
                    "candidate",
                    u64::from(candidate.sequence),
                ),
                &approved.project,
                candidate.sequence,
                candidate.maximum_training_seconds,
                candidate.parameters,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let budget = protocol_budget_from(
        &approved.proposal,
        deterministic.len(),
        source_protocol.development_suite_keys().len(),
    )?;

    let (candidates, adopted_protocol_id, adopted_run_id) = match &manifest.existing_experiment {
        Some(existing) => {
            let protocol = store
                .get_protocol(existing.protocol_id)
                .await?
                .context("manifest adoption protocol does not exist")?;
            protocol.validate_integrity(&approved.project)?;
            let view = load_persisted_view(existing.run_id, store).await?;
            if protocol.fingerprint != existing.protocol_fingerprint
                || view.protocol_id != protocol.id
                || view.last_event_fingerprint != existing.run_head_fingerprint
                || protocol.project_snapshot_id != approved.project.id
                || protocol.metric_contract != source_protocol.metric_contract
                || protocol.budget != budget
                || protocol.maximum_evaluation_seconds
                    != approved.proposal.budget.maximum_evaluation_seconds
                || protocol.development_suite_keys() != source_protocol.development_suite_keys()
                || protocol.sealed_suite_key != source_protocol.sealed_suite_key
                || protocol.development_selection_rule
                    != Some(DevelopmentSelectionRule::MaximizeWorstSuiteThenMean)
                || !candidate_shapes_match(&protocol.candidates, &deterministic)
                || !protocol_matches_benchmark(&protocol, &benchmark)
            {
                anyhow::bail!("existing experiment is not the exact approved optimization request");
            }
            (
                protocol.candidates,
                Some(existing.protocol_id),
                Some(existing.run_id),
            )
        }
        None => (deterministic, None, None),
    };
    let campaign_budget = campaign_budget_from(
        &approved.proposal,
        candidates.len(),
        source_protocol.development_suite_keys().len(),
    )?;
    let definition = ProductionOptimizationDefinition::create(
        manifest.name.clone(),
        manifest_fingerprint.clone(),
        &approved.project,
        OptimizationArtifactBinding::new(
            approved.proposal.id,
            approved.proposal.fingerprint.clone(),
        )?,
        OptimizationArtifactBinding::new(
            approved.selection.id,
            approved.selection.fingerprint.clone(),
        )?,
        OptimizationArtifactBinding::new(snapshot.id, snapshot.fingerprint.clone())?,
        snapshot.specification_fingerprint,
        benchmark,
        &source_protocol,
        candidates,
        campaign_budget,
        approved.proposal.budget.maximum_evaluation_seconds,
        Utc::now(),
    )?;
    Ok(ResolvedLaunch {
        manifest,
        manifest_fingerprint,
        definition,
        adopted_protocol_id,
        adopted_run_id,
    })
}

async fn attach_campaign(
    store: &SqliteExperimentStore,
    context: &LaunchContext,
) -> anyhow::Result<()> {
    ensure_generation_current(store, &context.definition).await?;
    let expected = ProductionCampaign::create_identified(
        context.run.reserved_campaign_id,
        format!("{} optimization", context.definition.name),
        context.definition.project.id,
        context.definition.project.fingerprint.clone(),
        context.definition.campaign_budget.clone(),
        context.run.created_at,
    )?;
    let campaign = match store.get_campaign(expected.id).await? {
        Some(existing) => {
            if existing != expected {
                anyhow::bail!("reserved campaign identity belongs to a different optimization");
            }
            existing
        }
        None => {
            let first = first_campaign_event(&expected, expected.created_at)?;
            store.create_campaign(&expected, &first).await?;
            expected
        }
    };
    let mut campaign_context = load_optimization_campaign(store, campaign.id).await?;
    if campaign_context.view.state == CampaignState::AwaitingGeneration {
        let event = bind_generation_event(
            &campaign_context.campaign,
            &campaign_context.view,
            context.definition.benchmark.clone(),
            Utc::now(),
        )?;
        store.append_campaign_event(&event).await?;
        campaign_context = load_optimization_campaign(store, campaign.id).await?;
    }
    let event = context.view.next_event(
        &context.run,
        OptimizationEventKind::CampaignAttached {
            campaign_fingerprint: campaign_context.campaign.fingerprint,
        },
        Utc::now(),
    )?;
    store.append_optimization_event(event).await?;
    Ok(())
}

async fn load_launch(store: &SqliteExperimentStore, run_id: Uuid) -> anyhow::Result<LaunchContext> {
    let (definition, run) = store
        .get_optimization_run(run_id)
        .await?
        .with_context(|| format!("production optimization run {run_id} does not exist"))?;
    let project = store
        .get_project(definition.project.id)
        .await?
        .context("optimization project does not exist")?;
    let metric_source = store
        .get_protocol(definition.metric_source_protocol.id)
        .await?
        .context("optimization metric source protocol does not exist")?;
    definition.validate_integrity(&project, &metric_source)?;
    run.validate_integrity(&definition)?;
    let events = store.list_optimization_events(run.id).await?;
    let view = replay_optimization(&run, &events)?;
    Ok(LaunchContext {
        definition,
        run,
        view,
    })
}

/// Cheap, fully deterministic campaign replay used by routine operator commands. Native project
/// inspection belongs to side-effecting runner stages and to `doctor`, not to every status poll.
async fn load_optimization_campaign(
    store: &SqliteExperimentStore,
    campaign_id: Uuid,
) -> anyhow::Result<CampaignContext> {
    let campaign = store
        .get_campaign(campaign_id)
        .await?
        .with_context(|| format!("production campaign {campaign_id} does not exist"))?;
    let project = store
        .get_project(campaign.project_snapshot_id)
        .await?
        .context("campaign project does not exist")?;
    if project.fingerprint != campaign.project_snapshot_fingerprint {
        anyhow::bail!("campaign project fingerprint changed");
    }
    let events = store.list_campaign_events(campaign.id).await?;
    let view = replay_campaign(&campaign, &events)?;
    let generation = match &view.current_generation {
        Some(binding) => {
            let value = load_generation(store, binding.generation_id).await?;
            if value.0.fingerprint != binding.generation_fingerprint {
                anyhow::bail!("campaign benchmark-generation binding changed");
            }
            Some(value)
        }
        None => None,
    };
    let experiment = match view.run_id {
        Some(run_id) => Some(load_persisted_view(run_id, store).await?),
        None => None,
    };
    Ok(CampaignContext {
        campaign,
        view,
        generation,
        experiment,
    })
}

async fn load_optional_campaign(
    store: &SqliteExperimentStore,
    context: &LaunchContext,
) -> anyhow::Result<Option<crate::commands::production_campaign::CampaignContext>> {
    if store
        .get_campaign(context.run.reserved_campaign_id)
        .await?
        .is_some()
    {
        let campaign = load_optimization_campaign(store, context.run.reserved_campaign_id).await?;
        if context
            .view
            .campaign_fingerprint
            .as_deref()
            .is_some_and(|value| value != campaign.campaign.fingerprint)
        {
            anyhow::bail!("optimization campaign binding changed");
        }
        Ok(Some(campaign))
    } else {
        Ok(None)
    }
}

async fn ensure_generation_current(
    store: &SqliteExperimentStore,
    definition: &ProductionOptimizationDefinition,
) -> anyhow::Result<()> {
    ensure_generation_matches(store, definition, true).await
}

async fn ensure_generation_matches(
    store: &SqliteExperimentStore,
    definition: &ProductionOptimizationDefinition,
    require_active: bool,
) -> anyhow::Result<()> {
    let (generation, view) = load_generation(store, definition.benchmark.generation_id).await?;
    if generation.fingerprint != definition.benchmark.generation_fingerprint {
        anyhow::bail!("optimization benchmark generation fingerprint changed");
    }
    if require_active {
        require_active_generation(&view, &generation)?;
    }
    Ok(())
}

fn require_active_generation(
    view: &BenchmarkGenerationView,
    generation: &workflow_core::benchmark_generation::BenchmarkGeneration,
) -> anyhow::Result<()> {
    if view.state != BenchmarkGenerationState::Active || !view.is_adaptive_eligible() {
        anyhow::bail!("optimization benchmark generation is not active and unused");
    }
    generation
        .freshness
        .as_ref()
        .context("optimization benchmark generation has no renewable freshness authority")?
        .validate_at(Utc::now())?;
    Ok(())
}

fn protocol_budget(
    definition: &ProductionOptimizationDefinition,
) -> anyhow::Result<OptimizationBudget> {
    Ok(OptimizationBudget {
        maximum_candidates: definition.campaign_budget.maximum_candidates,
        maximum_training_seconds: definition.campaign_budget.maximum_training_seconds,
        maximum_development_evaluations: definition.campaign_budget.maximum_development_evaluations,
        maximum_sealed_evaluations: definition.campaign_budget.maximum_sealed_evaluations,
    })
}

fn protocol_budget_from(
    proposal: &RepairProposal,
    candidate_count: usize,
    suite_count: usize,
) -> anyhow::Result<OptimizationBudget> {
    let candidate_count = u32::try_from(candidate_count)?;
    let suite_count = u32::try_from(suite_count)?;
    let development = candidate_count
        .checked_mul(suite_count)
        .context("development evaluation budget overflowed")?;
    if development > proposal.budget.maximum_development_evaluations
        || proposal.budget.maximum_sealed_uses != 1
    {
        anyhow::bail!("repair budget cannot cover the exact optimization protocol");
    }
    Ok(OptimizationBudget {
        maximum_candidates: candidate_count,
        maximum_training_seconds: proposal.budget.maximum_training_seconds,
        maximum_development_evaluations: development,
        maximum_sealed_evaluations: 1,
    })
}

fn campaign_budget_from(
    proposal: &RepairProposal,
    candidate_count: usize,
    suite_count: usize,
) -> anyhow::Result<CampaignBudget> {
    let candidate_count = u32::try_from(candidate_count)?;
    let suite_count = u32::try_from(suite_count)?;
    let development = candidate_count
        .checked_mul(suite_count)
        .context("development evaluation budget overflowed")?;
    let backend_operations = suite_count
        .checked_add(1)
        .and_then(|value| value.checked_add(candidate_count))
        .and_then(|value| value.checked_add(development))
        .and_then(|value| value.checked_add(1))
        .context("backend-operation budget overflowed")?;
    Ok(CampaignBudget {
        maximum_iterations: 1,
        maximum_candidates: candidate_count,
        maximum_training_seconds: proposal.budget.maximum_training_seconds,
        maximum_development_evaluations: development,
        maximum_sealed_evaluations: 1,
        maximum_backend_operations: backend_operations,
    })
}

fn candidate_shapes_match(existing: &[TrainingCandidate], expected: &[TrainingCandidate]) -> bool {
    existing.len() == expected.len()
        && existing.iter().zip(expected).all(|(left, right)| {
            left.project_snapshot_id == right.project_snapshot_id
                && left.project_snapshot_fingerprint == right.project_snapshot_fingerprint
                && left.sequence == right.sequence
                && left.maximum_training_seconds == right.maximum_training_seconds
                && left.parameters == right.parameters
        })
}

fn protocol_matches_benchmark(
    protocol: &ExperimentProtocol,
    benchmark: &CampaignBenchmarkBinding,
) -> bool {
    protocol
        .baseline_development_reports()
        .into_iter()
        .map(|report| (report.suite_key.clone(), report.suite_fingerprint.clone()))
        .collect::<std::collections::BTreeMap<_, _>>()
        == benchmark.development_suite_fingerprints
        && protocol.sealed_suite_key == benchmark.sealed_suite_key
        && protocol.baseline_sealed_report.suite_fingerprint == benchmark.sealed_suite_fingerprint
}

fn deterministic_uuid(manifest_fingerprint: &str, kind: &str, sequence: u64) -> Uuid {
    let digest = Sha256::digest(format!("{manifest_fingerprint}:{kind}:{sequence}").as_bytes());
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn read_manifest(path: &Path) -> anyhow::Result<OptimizeManifest> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("could not inspect optimization manifest {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        anyhow::bail!("optimization manifest must be a regular file no larger than 1 MiB");
    }
    let bytes = fs::read(path)
        .with_context(|| format!("could not read optimization manifest {}", path.display()))?;
    let text = std::str::from_utf8(&bytes).context("optimization manifest is not UTF-8")?;
    toml::from_str(text).context("optimization manifest is not valid strict TOML")
}

fn validate_manifest(manifest: &OptimizeManifest) -> anyhow::Result<()> {
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION
        || manifest.name.trim() != manifest.name
        || manifest.name.is_empty()
        || manifest.name.chars().count() > 120
        || manifest.project_source_revision.trim() != manifest.project_source_revision
        || manifest.project_source_revision.is_empty()
        || manifest.training_snapshot_id.is_nil()
        || manifest.benchmark_generation_id.is_nil()
        || !canonical_fingerprint(&manifest.training_snapshot_fingerprint)
        || !canonical_fingerprint(&manifest.training_snapshot_specification_fingerprint)
        || !canonical_fingerprint(&manifest.benchmark_generation_fingerprint)
        || manifest.selection_policy != DevelopmentSelectionRule::MaximizeWorstSuiteThenMean
    {
        anyhow::bail!("optimization manifest is incomplete or uses unsupported policy values");
    }
    if let Some(existing) = &manifest.existing_experiment {
        if existing.protocol_id.is_nil()
            || existing.run_id.is_nil()
            || !canonical_fingerprint(&existing.protocol_fingerprint)
            || !canonical_fingerprint(&existing.run_head_fingerprint)
        {
            anyhow::bail!("optimization manifest existing experiment binding is incomplete");
        }
    }
    Ok(())
}

fn canonical_fingerprint(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .chars()
                .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
    })
}

async fn print_current_status(
    store: &SqliteExperimentStore,
    context: &LaunchContext,
    existing: bool,
) -> anyhow::Result<()> {
    let campaign = load_optional_campaign(store, context).await?;
    print_status(context, existing, campaign.as_ref())
}

fn print_status(
    context: &LaunchContext,
    existing: bool,
    campaign: Option<&crate::commands::production_campaign::CampaignContext>,
) -> anyhow::Result<()> {
    let campaign_view = campaign.map(|value| &value.view);
    let experiment = campaign.and_then(|value| value.experiment.as_ref());
    let mut completed = Vec::new();
    if campaign.is_some() {
        completed.push("campaign_reserved_and_attached");
    }
    if campaign_view.and_then(|value| value.protocol_id).is_some() {
        completed.push("protocol_prepared");
    }
    if campaign_view.and_then(|value| value.run_id).is_some() {
        completed.push("experiment_run_started_or_adopted");
    }
    if campaign_view.is_some_and(|value| {
        matches!(
            value.state,
            CampaignState::AwaitingSealedAuthorization
                | CampaignState::SealedAuthorized
                | CampaignState::AwaitingFinalization
                | CampaignState::RenewalRequired
                | CampaignState::Completed
        )
    }) {
        completed.push("development_evaluation");
    }
    if experiment.is_some_and(|value| value.sealed_report.is_some()) {
        completed.push("sealed_evaluation");
    }
    if context.view.state == OptimizationRunState::Completed {
        completed.push("final_decision_persisted");
    }
    let requires_human_authorization = campaign_view
        .is_some_and(|value| value.state == CampaignState::AwaitingSealedAuthorization);
    let stopped_reason = match context.view.state {
        OptimizationRunState::Cancelled => "cancelled_by_operator",
        OptimizationRunState::Failed => "failed",
        OptimizationRunState::Completed => "finite_run_completed",
        OptimizationRunState::Planned => "waiting_for_operator_resume",
        OptimizationRunState::CampaignActive if requires_human_authorization => {
            "explicit_sealed_use_authorization_required"
        }
        OptimizationRunState::CampaignActive => "stage_boundary_waiting_for_operator_resume",
    };
    let reserved = campaign_view.map(|value| value.reserved_usage);
    let budget = &context.definition.campaign_budget;
    let remaining = reserved.map(|value| serde_json::json!({
        "iterations": budget.maximum_iterations.saturating_sub(value.iterations),
        "candidates": budget.maximum_candidates.saturating_sub(value.candidates),
        "training_seconds": budget.maximum_training_seconds.saturating_sub(value.training_seconds),
        "development_evaluations": budget.maximum_development_evaluations.saturating_sub(value.development_evaluations),
        "sealed_evaluations": budget.maximum_sealed_evaluations.saturating_sub(value.sealed_evaluations),
        "backend_operations": budget.maximum_backend_operations.saturating_sub(value.backend_operations),
    }));
    presentation::print(&serde_json::json!({
        "run_id": context.run.id,
        "existing": existing,
        "state": context.view.state,
        "campaign_state": campaign_view.map(|value| value.state),
        "experiment_state": experiment.map(|value| value.state),
        "decision": context.view.decision,
        "selected_candidate_id": experiment.and_then(|value| value.selected_candidate_id),
        "completed": completed,
        "failed_or_uncertain": context.view.failure_reason,
        "stopped_reason": stopped_reason,
        "human_authorization_required": requires_human_authorization,
        "last_sequence": context.view.last_sequence,
        "head_fingerprint": context.view.last_event_fingerprint,
        "artifacts": {
            "definition_id": context.definition.id,
            "training_snapshot_id": context.definition.training_snapshot.id,
            "campaign_id": context.run.reserved_campaign_id,
            "protocol_id": context.run.reserved_protocol_id,
            "experiment_run_id": context.run.reserved_experiment_run_id,
        },
        "budgets": {
            "maximum": budget,
            "reserved": reserved,
            "remaining_unreserved": remaining,
        },
        "next_command": next_command(context, campaign),
    }))
}

fn next_command(
    context: &LaunchContext,
    campaign: Option<&crate::commands::production_campaign::CampaignContext>,
) -> &'static str {
    match context.view.state {
        OptimizationRunState::Planned => "resume",
        OptimizationRunState::Completed
        | OptimizationRunState::Cancelled
        | OptimizationRunState::Failed => "none",
        OptimizationRunState::CampaignActive => match campaign.map(|value| value.view.state) {
            Some(CampaignState::AwaitingSealedAuthorization) => "authorize-sealed",
            _ => "resume",
        },
    }
}

pub(crate) fn backend_args(command: &EncoderOptimizeCommand) -> &NomosWorkspaceArgs {
    match command {
        EncoderOptimizeCommand::Preview(args) | EncoderOptimizeCommand::Start(args) => {
            &args.backend
        }
        EncoderOptimizeCommand::Status(args)
        | EncoderOptimizeCommand::Inspect(args)
        | EncoderOptimizeCommand::ReviewRepair(args)
        | EncoderOptimizeCommand::ReviewDelta(args)
        | EncoderOptimizeCommand::Resume(args)
        | EncoderOptimizeCommand::Doctor(args)
        | EncoderOptimizeCommand::Provenance(args)
        | EncoderOptimizeCommand::Report(args) => &args.backend,
        EncoderOptimizeCommand::AuthorizeSealed(args) => &args.backend,
        EncoderOptimizeCommand::AuthorizeExternal(args) => &args.backend,
        EncoderOptimizeCommand::Cancel(args) => &args.backend,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_manifest_rejects_unknown_fields() {
        let text = r#"
schema_version = 1
name = "bounded repair"
adapter = "nomos"
project_source_revision = "abc"
training_snapshot_id = "00000000-0000-4000-8000-000000000001"
training_snapshot_fingerprint = "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
training_snapshot_specification_fingerprint = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
benchmark_generation_id = "00000000-0000-4000-8000-000000000002"
benchmark_generation_fingerprint = "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
approval_mode = "explicit_sealed_use"
selection_policy = "maximize_worst_suite_then_mean"
final_decision_policy = "strict_metric_contract"
surprise = true
"#;
        assert!(toml::from_str::<OptimizeManifest>(text).is_err());
    }

    #[test]
    fn reserved_candidate_ids_are_stable_and_distinct() {
        let first = deterministic_uuid("sha256:test", "candidate", 1);
        assert_eq!(first, deterministic_uuid("sha256:test", "candidate", 1));
        assert_ne!(first, deterministic_uuid("sha256:test", "candidate", 2));
        assert!(!first.is_nil());
    }
}
