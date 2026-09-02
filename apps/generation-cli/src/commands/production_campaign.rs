use std::path::Path;

use anyhow::Context;
use chrono::Utc;
use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget, CampaignState, CampaignStore, CampaignView,
    ProductionCampaign, SealedAssessmentExposure, bind_generation_event, complete_campaign_event,
    finalize_iteration_event, first_campaign_event, prepare_iteration_event,
    record_development_event, record_sealed_authorization_event, renewal_handoff_event,
    replay_campaign, start_run_event,
};
use encoder_experiment_core::{
    journal::{ExperimentRunState, ExperimentView},
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use serde::Deserialize;
use uuid::Uuid;
use workflow_core::{
    benchmark_generation::{
        BenchmarkGeneration, BenchmarkGenerationEventKind, BenchmarkGenerationState,
        BenchmarkGenerationView, first_generation_event, prepare_successor_activation,
        replay_benchmark_generation,
    },
    ports::BenchmarkGenerationStore,
};

use crate::{
    cli::{
        BenchmarkGenerationCommand, BenchmarkGenerationIdArgs, NomosWorkspaceArgs,
        ProductionCampaignCommand, ProductionCampaignIdArgs,
    },
    commands::experiment::{ensure_database_belongs_to_workspace, prepare_protocol_from_file},
    presentation,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CampaignCreateInput {
    name: String,
    budget: CampaignBudget,
}

pub async fn execute_generation(
    command: BenchmarkGenerationCommand,
    database_url: &str,
) -> anyhow::Result<()> {
    let backend = generation_backend_args(&command);
    ensure_database_belongs_to_workspace(database_url, &backend.workspace)?;
    let store = SqliteExperimentStore::connect(database_url).await?;
    match command {
        BenchmarkGenerationCommand::Import(args) => {
            let generation: BenchmarkGeneration = read_strict_json(&args.file, 4 * 1_048_576)?;
            generation.validate_integrity()?;
            if let Some(predecessor_id) = generation.predecessor_id {
                let predecessor = store
                    .get_benchmark_generation(predecessor_id)
                    .await?
                    .with_context(|| {
                        format!("predecessor benchmark generation {predecessor_id} does not exist")
                    })?;
                if generation.predecessor_fingerprint.as_deref()
                    != Some(predecessor.fingerprint.as_str())
                {
                    anyhow::bail!("generation predecessor fingerprint does not match persistence");
                }
            }
            let first = first_generation_event(&generation, generation.created_at)?;
            store
                .create_benchmark_generation(&generation, &first)
                .await?;
            print_generation(
                &generation,
                &replay_benchmark_generation(&generation, &[first])?,
            )
        }
        BenchmarkGenerationCommand::Show(args) => {
            let (generation, view) = load_generation(&store, args.generation_id).await?;
            print_generation(&generation, &view)
        }
        BenchmarkGenerationCommand::MarkReady(args) => {
            let (generation, view) = load_generation(&store, args.generation_id).await?;
            let event = view.next_event(
                &generation,
                BenchmarkGenerationEventKind::MarkedReady {
                    confirmed_by: args.actor,
                },
                Utc::now(),
            )?;
            store.append_benchmark_generation_event(&event).await?;
            let (_, view) = load_generation(&store, generation.id).await?;
            print_generation(&generation, &view)
        }
        BenchmarkGenerationCommand::ActivateInitial(args) => {
            let (generation, view) = load_generation(&store, args.generation_id).await?;
            if generation.predecessor_id.is_some() {
                anyhow::bail!(
                    "a successor must use activate-successor so predecessor retirement is atomic"
                );
            }
            let event = view.next_event(
                &generation,
                BenchmarkGenerationEventKind::Activated {
                    activated_by: args.actor,
                },
                Utc::now(),
            )?;
            store.append_benchmark_generation_event(&event).await?;
            let (_, view) = load_generation(&store, generation.id).await?;
            print_generation(&generation, &view)
        }
        BenchmarkGenerationCommand::ActivateSuccessor(args) => {
            let (predecessor, predecessor_view) =
                load_generation(&store, args.predecessor_generation_id).await?;
            let (successor, successor_view) =
                load_generation(&store, args.successor_generation_id).await?;
            let activation = prepare_successor_activation(
                &predecessor,
                &predecessor_view,
                &successor,
                &successor_view,
                args.actor,
                Utc::now(),
            )?;
            store
                .activate_successor_generation(
                    &activation.predecessor_superseded,
                    &activation.successor_activated,
                )
                .await?;
            let (_, view) = load_generation(&store, successor.id).await?;
            print_generation(&successor, &view)
        }
        BenchmarkGenerationCommand::Exhaust(args) => {
            let (generation, view) = load_generation(&store, args.generation_id).await?;
            let event = view.next_event(
                &generation,
                BenchmarkGenerationEventKind::Exhausted {
                    reason: args.reason,
                },
                Utc::now(),
            )?;
            store.append_benchmark_generation_event(&event).await?;
            let (_, view) = load_generation(&store, generation.id).await?;
            print_generation(&generation, &view)
        }
    }
}

pub async fn execute_campaign(
    command: ProductionCampaignCommand,
    database_url: &str,
) -> anyhow::Result<()> {
    let backend_args = campaign_backend_args(&command);
    ensure_database_belongs_to_workspace(database_url, &backend_args.workspace)?;
    let store = SqliteExperimentStore::connect(database_url).await?;
    let backend = NomosBackend::open(&backend_args.workspace, backend_args.python.clone())?;
    let runner = ExperimentRunner::new(&store, &backend);

    match command {
        ProductionCampaignCommand::Create(args) => {
            let input: CampaignCreateInput = read_strict_json(&args.file, 1_048_576)?;
            let project = store.get_project(args.project_id).await?.with_context(|| {
                format!("experiment project {} does not exist", args.project_id)
            })?;
            backend.inspect(project.clone()).await?;
            let now = Utc::now();
            let campaign = ProductionCampaign::create(
                input.name,
                project.id,
                project.fingerprint,
                input.budget,
                now,
            )?;
            let first = first_campaign_event(&campaign, now)?;
            store.create_campaign(&campaign, &first).await?;
            print_campaign(
                &campaign,
                &replay_campaign(&campaign, &[first])?,
                None,
                None,
            )
        }
        ProductionCampaignCommand::Show(args) => {
            let context = load_campaign_context(&store, &backend, args.campaign_id).await?;
            print_campaign(
                &context.campaign,
                &context.view,
                context.generation.as_ref().map(|value| &value.1),
                context.experiment.as_ref(),
            )
        }
        ProductionCampaignCommand::Readiness(args) => {
            let context = load_campaign_context(&store, &backend, args.campaign_id).await?;
            print_readiness(&context)
        }
        ProductionCampaignCommand::BindGeneration(args) => {
            let context = load_campaign_context(&store, &backend, args.campaign_id).await?;
            let (generation, generation_view) = load_generation(&store, args.generation_id).await?;
            let binding = CampaignBenchmarkBinding::from_active_generation(
                &generation,
                &generation_view,
                args.sealed_suite_key,
            )?;
            let event =
                bind_generation_event(&context.campaign, &context.view, binding, Utc::now())?;
            store.append_campaign_event(&event).await?;
            print_reloaded_campaign(&store, &backend, context.campaign.id).await
        }
        ProductionCampaignCommand::Prepare(args) => {
            let context = load_campaign_context(&store, &backend, args.campaign_id).await?;
            if context.view.state != CampaignState::ReadyToPrepare {
                anyhow::bail!("campaign is not ready to prepare an experiment iteration");
            }
            ensure_bound_generation_eligible(&context)?;
            let protocol = prepare_protocol_from_file(
                context.campaign.project_snapshot_id,
                &args.file,
                &runner,
                &store,
            )
            .await?;
            let event =
                prepare_iteration_event(&context.campaign, &context.view, &protocol, Utc::now())?;
            store.append_campaign_event(&event).await?;
            print_reloaded_campaign(&store, &backend, context.campaign.id).await
        }
        ProductionCampaignCommand::Start(args) => {
            start_campaign_iteration(&store, &backend, &runner, args.campaign_id).await?;
            print_reloaded_campaign(&store, &backend, args.campaign_id).await
        }
        ProductionCampaignCommand::Advance(args) => {
            advance_campaign(&store, &backend, &runner, args.campaign_id).await?;
            print_reloaded_campaign(&store, &backend, args.campaign_id).await
        }
        ProductionCampaignCommand::AuthorizeSealed(args) => {
            let context = load_campaign_context(&store, &backend, args.campaign_id).await?;
            if context.view.state != CampaignState::AwaitingSealedAuthorization {
                anyhow::bail!("campaign is not waiting for sealed-use authorization");
            }
            let run_id = context.view.run_id.context("campaign has no current run")?;
            let experiment = runner.authorize_sealed(run_id, &args.authorized_by).await?;
            let event = record_sealed_authorization_event(
                &context.campaign,
                &context.view,
                &experiment,
                args.authorized_by,
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
            print_reloaded_campaign(&store, &backend, context.campaign.id).await
        }
        ProductionCampaignCommand::LinkRenewalHandoff(args) => {
            let context = load_campaign_context(&store, &backend, args.campaign_id).await?;
            let event = renewal_handoff_event(
                &context.campaign,
                &context.view,
                args.handoff_id,
                args.handoff_fingerprint,
                Utc::now(),
            )?;
            store.append_campaign_event(&event).await?;
            print_reloaded_campaign(&store, &backend, context.campaign.id).await
        }
        ProductionCampaignCommand::Complete(args) => {
            let context = load_campaign_context(&store, &backend, args.campaign_id).await?;
            let event =
                complete_campaign_event(&context.campaign, &context.view, args.reason, Utc::now())?;
            store.append_campaign_event(&event).await?;
            print_reloaded_campaign(&store, &backend, context.campaign.id).await
        }
    }
}

struct CampaignContext {
    campaign: ProductionCampaign,
    view: CampaignView,
    generation: Option<(BenchmarkGeneration, BenchmarkGenerationView)>,
    experiment: Option<ExperimentView>,
}

async fn load_campaign_context(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
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
    backend.inspect(project).await?;
    let events = store.list_campaign_events(campaign.id).await?;
    let view = replay_campaign(&campaign, &events)?;
    let generation = match &view.current_generation {
        Some(binding) => {
            let (generation, generation_view) =
                load_generation(store, binding.generation_id).await?;
            if generation.fingerprint != binding.generation_fingerprint {
                anyhow::bail!("campaign benchmark-generation binding changed");
            }
            Some((generation, generation_view))
        }
        None => None,
    };
    let experiment = match view.run_id {
        Some(run_id) => Some(ExperimentRunner::new(store, backend).status(run_id).await?),
        None => None,
    };
    Ok(CampaignContext {
        campaign,
        view,
        generation,
        experiment,
    })
}

async fn load_generation(
    store: &SqliteExperimentStore,
    generation_id: Uuid,
) -> anyhow::Result<(BenchmarkGeneration, BenchmarkGenerationView)> {
    let generation = store
        .get_benchmark_generation(generation_id)
        .await?
        .with_context(|| format!("benchmark generation {generation_id} does not exist"))?;
    let events = store
        .list_benchmark_generation_events(generation_id)
        .await?;
    let view = replay_benchmark_generation(&generation, &events)?;
    Ok((generation, view))
}

fn ensure_bound_generation_eligible(context: &CampaignContext) -> anyhow::Result<()> {
    let generation = context
        .generation
        .as_ref()
        .context("campaign has no benchmark generation")?;
    if !generation.1.is_adaptive_eligible() {
        anyhow::bail!("bound benchmark generation is exhausted or inactive");
    }
    generation.0.freshness.validate_at(Utc::now())?;
    Ok(())
}

async fn start_campaign_iteration(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    runner: &ExperimentRunner<'_, SqliteExperimentStore, NomosBackend>,
    campaign_id: Uuid,
) -> anyhow::Result<()> {
    let context = load_campaign_context(store, backend, campaign_id).await?;
    if context.view.state != CampaignState::ReadyToStart {
        anyhow::bail!("campaign is not ready to start its prepared iteration");
    }
    ensure_bound_generation_eligible(&context)?;
    let protocol_id = context
        .view
        .protocol_id
        .context("campaign has no protocol")?;
    let existing = store.run_ids_for_protocol(protocol_id).await?;
    let experiment = match existing.as_slice() {
        [] => runner.create_run(protocol_id).await?,
        [run_id] => runner.status(*run_id).await?,
        _ => anyhow::bail!("prepared campaign protocol has multiple experiment runs"),
    };
    let event = start_run_event(&context.campaign, &context.view, &experiment, Utc::now())?;
    store.append_campaign_event(&event).await?;
    Ok(())
}

async fn advance_campaign(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    runner: &ExperimentRunner<'_, SqliteExperimentStore, NomosBackend>,
    campaign_id: Uuid,
) -> anyhow::Result<()> {
    for _ in 0..4 {
        let context = load_campaign_context(store, backend, campaign_id).await?;
        match context.view.state {
            CampaignState::ReadyToStart => {
                start_campaign_iteration(store, backend, runner, campaign_id).await?;
            }
            CampaignState::RunningDevelopment => {
                ensure_bound_generation_eligible(&context)?;
                let run_id = context.view.run_id.context("campaign has no current run")?;
                let current = context
                    .experiment
                    .context("campaign experiment is missing")?;
                let experiment = if matches!(
                    current.state,
                    ExperimentRunState::AwaitingSealedAuthorization | ExperimentRunState::Completed
                ) {
                    current
                } else {
                    runner.run_development(run_id).await?
                };
                let event = record_development_event(
                    &context.campaign,
                    &context.view,
                    &experiment,
                    Utc::now(),
                )?;
                store.append_campaign_event(&event).await?;
            }
            CampaignState::SealedAuthorized => {
                let run_id = context.view.run_id.context("campaign has no current run")?;
                let experiment = match context.experiment.as_ref() {
                    Some(value) if value.state == ExperimentRunState::Completed => value.clone(),
                    _ => runner.run_sealed(run_id).await?,
                };
                finalize_campaign_iteration(store, &context, &experiment).await?;
            }
            CampaignState::AwaitingFinalization => {
                let experiment = context
                    .experiment
                    .as_ref()
                    .context("campaign run is missing")?;
                finalize_campaign_iteration(store, &context, experiment).await?;
            }
            CampaignState::AwaitingSealedAuthorization => {
                eprintln!(
                    "stopped: explicit sealed-use authorization is required; run `synth production-campaign authorize-sealed {campaign_id}`"
                );
                return Ok(());
            }
            CampaignState::AwaitingGeneration
            | CampaignState::ReadyToPrepare
            | CampaignState::RenewalRequired
            | CampaignState::Completed => return Ok(()),
        }
    }
    anyhow::bail!("campaign advance exceeded its deterministic transition bound")
}

async fn finalize_campaign_iteration(
    store: &SqliteExperimentStore,
    context: &CampaignContext,
    experiment: &ExperimentView,
) -> anyhow::Result<()> {
    let (generation, generation_view) = context
        .generation
        .as_ref()
        .context("campaign has no generation")?;
    if generation_view.state != BenchmarkGenerationState::Active {
        anyhow::bail!("campaign generation is already exhausted or inactive");
    }
    let now = Utc::now();
    let exposure = if experiment.selected_candidate_id.is_some() {
        Some(SealedAssessmentExposure::from_completed_experiment(
            &context.campaign,
            &context.view,
            experiment,
            now,
        )?)
    } else {
        None
    };
    let terminal = generation_view.next_event(
        generation,
        match &exposure {
            Some(exposure) => BenchmarkGenerationEventKind::IterationConsumed {
                experiment_run_id: experiment.run_id,
                experiment_protocol_fingerprint: context
                    .view
                    .protocol_fingerprint
                    .clone()
                    .context("campaign protocol fingerprint is missing")?,
                sealed_exposure_id: exposure.id,
                sealed_exposure_fingerprint: exposure.fingerprint.clone(),
                final_decision_fingerprint: experiment.last_event_fingerprint.clone(),
            },
            None => BenchmarkGenerationEventKind::Exhausted {
                reason: "no candidate passed every development suite".into(),
            },
        },
        now,
    )?;
    let campaign_event = finalize_iteration_event(
        &context.campaign,
        &context.view,
        experiment,
        &terminal,
        exposure,
        now,
    )?;
    store
        .append_iteration_finalization(&terminal, &campaign_event)
        .await?;
    Ok(())
}

fn print_generation(
    generation: &BenchmarkGeneration,
    view: &BenchmarkGenerationView,
) -> anyhow::Result<()> {
    presentation::print(&serde_json::json!({
        "generation": generation,
        "lifecycle": view,
        "adaptive_eligible": view.is_adaptive_eligible(),
        "fresh_at_command_time": generation.freshness.validate_at(Utc::now()).is_ok(),
    }))
}

fn print_campaign(
    campaign: &ProductionCampaign,
    view: &CampaignView,
    generation: Option<&BenchmarkGenerationView>,
    experiment: Option<&ExperimentView>,
) -> anyhow::Result<()> {
    presentation::print(&serde_json::json!({
        "campaign": campaign,
        "lifecycle": view,
        "benchmark_generation_lifecycle": generation,
        "experiment": experiment,
        "readiness": readiness_value(view, generation, experiment),
    }))
}

async fn print_reloaded_campaign(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    campaign_id: Uuid,
) -> anyhow::Result<()> {
    let context = load_campaign_context(store, backend, campaign_id).await?;
    print_campaign(
        &context.campaign,
        &context.view,
        context.generation.as_ref().map(|value| &value.1),
        context.experiment.as_ref(),
    )
}

fn print_readiness(context: &CampaignContext) -> anyhow::Result<()> {
    presentation::print(&readiness_value(
        &context.view,
        context.generation.as_ref().map(|value| &value.1),
        context.experiment.as_ref(),
    ))
}

fn readiness_value(
    view: &CampaignView,
    generation: Option<&BenchmarkGenerationView>,
    experiment: Option<&ExperimentView>,
) -> serde_json::Value {
    let (ready, next_action, boundary) = match view.state {
        CampaignState::AwaitingGeneration => (false, "bind_generation", "approved_successor"),
        CampaignState::ReadyToPrepare => (true, "prepare", "operator_protocol"),
        CampaignState::ReadyToStart => (true, "start_or_advance", "none"),
        CampaignState::RunningDevelopment => (true, "advance", "none"),
        CampaignState::AwaitingSealedAuthorization => {
            (false, "authorize_sealed", "explicit_sealed_use")
        }
        CampaignState::SealedAuthorized => (true, "advance", "none"),
        CampaignState::AwaitingFinalization => (true, "advance", "none"),
        CampaignState::RenewalRequired => (false, "renew_or_complete", "fresh_successor"),
        CampaignState::Completed => (false, "none", "campaign_complete"),
    };
    serde_json::json!({
        "state": view.state,
        "ready_for_automatic_advance": ready,
        "next_action": next_action,
        "authority_boundary": boundary,
        "generation_state": generation.map(|value| value.state),
        "generation_adaptive_eligible": generation.is_some_and(BenchmarkGenerationView::is_adaptive_eligible),
        "experiment_state": experiment.map(|value| value.state),
    })
}

fn read_strict_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    maximum_bytes: u64,
) -> anyhow::Result<T> {
    let metadata = path
        .metadata()
        .with_context(|| format!("could not inspect {}", path.display()))?;
    if !metadata.is_file() || metadata.len() > maximum_bytes {
        anyhow::bail!(
            "{} must be a file of at most {} bytes",
            path.display(),
            maximum_bytes
        );
    }
    let bytes =
        std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is invalid strict JSON", path.display()))
}

fn generation_backend_args(command: &BenchmarkGenerationCommand) -> &NomosWorkspaceArgs {
    match command {
        BenchmarkGenerationCommand::Import(args) => &args.backend,
        BenchmarkGenerationCommand::Show(BenchmarkGenerationIdArgs { backend, .. }) => backend,
        BenchmarkGenerationCommand::MarkReady(args)
        | BenchmarkGenerationCommand::ActivateInitial(args) => &args.backend,
        BenchmarkGenerationCommand::ActivateSuccessor(args) => &args.backend,
        BenchmarkGenerationCommand::Exhaust(args) => &args.backend,
    }
}

fn campaign_backend_args(command: &ProductionCampaignCommand) -> &NomosWorkspaceArgs {
    match command {
        ProductionCampaignCommand::Create(args) => &args.backend,
        ProductionCampaignCommand::Show(ProductionCampaignIdArgs { backend, .. })
        | ProductionCampaignCommand::Readiness(ProductionCampaignIdArgs { backend, .. })
        | ProductionCampaignCommand::Start(ProductionCampaignIdArgs { backend, .. })
        | ProductionCampaignCommand::Advance(ProductionCampaignIdArgs { backend, .. }) => backend,
        ProductionCampaignCommand::BindGeneration(args) => &args.backend,
        ProductionCampaignCommand::Prepare(args) => &args.backend,
        ProductionCampaignCommand::AuthorizeSealed(args) => &args.backend,
        ProductionCampaignCommand::LinkRenewalHandoff(args) => &args.backend,
        ProductionCampaignCommand::Complete(args) => &args.backend,
    }
}
