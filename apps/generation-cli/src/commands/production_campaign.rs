use std::{
    fs::OpenOptions,
    io::Write,
    path::{Component, Path},
};

use anyhow::Context;
use chrono::Utc;
use encoder_campaign_core::{
    CampaignBenchmarkBinding, CampaignBudget, CampaignEventKind, CampaignState, CampaignStore,
    CampaignView, ProductionCampaign, SealedAssessmentExposure, bind_generation_event,
    complete_campaign_event, finalize_iteration_event, first_campaign_event,
    prepare_iteration_event, record_development_event, record_sealed_authorization_event,
    renewal_handoff_event, replay_campaign, start_run_event,
};
use encoder_experiment_core::{
    journal::{ExperimentRunState, ExperimentView, replay_experiment},
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use serde::Deserialize;
use sha2::Digest;
use uuid::Uuid;
use workflow_core::{
    benchmark_generation::{
        BenchmarkGeneration, BenchmarkGenerationEventKind, BenchmarkGenerationState,
        BenchmarkGenerationView, HistoricalBenchmarkConsumption, first_generation_event,
        prepare_successor_activation, replay_benchmark_generation,
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
    let store = command.database_access().production(database_url).await?;
    match command {
        BenchmarkGenerationCommand::NomosBuildAuthority(args) => {
            if args.valid_days <= 0 || args.valid_days > 365 {
                anyhow::bail!("authority validity must be between 1 and 365 days");
            }
            if args.output.is_absolute()
                || args.output.components().any(|part| {
                    matches!(
                        part,
                        Component::ParentDir | Component::RootDir | Component::Prefix(_)
                    )
                })
            {
                anyhow::bail!("authority output must be a relative path inside the workspace");
            }
            let workspace = args.backend.workspace.canonicalize().with_context(|| {
                format!("could not resolve {}", args.backend.workspace.display())
            })?;
            let output = workspace.join(&args.output);
            let parent = output.parent().context("authority output has no parent")?;
            let canonical_parent = parent
                .canonicalize()
                .with_context(|| format!("could not resolve {}", parent.display()))?;
            if !canonical_parent.starts_with(&workspace) {
                anyhow::bail!("authority output escapes the isolated workspace");
            }
            let now = Utc::now();
            let backend = NomosBackend::open(&workspace, args.backend.python)?;
            let bytes = backend.build_benchmark_authority(
                &args.qualification_report,
                args.acquired_by,
                args.reviewed_by,
                args.rationale,
                now,
                now + chrono::Duration::days(args.valid_days),
            )?;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&output)
                .with_context(|| {
                    format!(
                        "could not create {}; authority files are never overwritten",
                        output.display()
                    )
                })?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            let sha256 = format!("{:x}", sha2::Sha256::digest(&bytes));
            presentation::print(&serde_json::json!({
                "created": true,
                "path": args.output,
                "bytes": bytes.len(),
                "sha256": sha256,
                "valid_until": now + chrono::Duration::days(args.valid_days),
            }))
        }
        BenchmarkGenerationCommand::MigrateConsumed(args) => {
            let events = store.load_events(args.experiment_run_id).await?;
            let first = events.first().with_context(|| {
                format!(
                    "experiment run {} has no persisted journal",
                    args.experiment_run_id
                )
            })?;
            if first.run_id != args.experiment_run_id {
                anyhow::bail!("experiment run journal identity does not match the request");
            }
            let protocol = store
                .get_protocol(first.protocol_id)
                .await?
                .with_context(|| {
                    format!("experiment protocol {} does not exist", first.protocol_id)
                })?;
            if protocol.fingerprint != first.protocol_fingerprint {
                anyhow::bail!("experiment protocol fingerprint does not match the run journal");
            }
            let project = store
                .get_project(protocol.project_snapshot_id)
                .await?
                .with_context(|| {
                    format!(
                        "experiment project {} does not exist",
                        protocol.project_snapshot_id
                    )
                })?;
            let backend = NomosBackend::open(&args.backend.workspace, args.backend.python)?;
            backend.inspect(project.clone()).await?;
            let view = replay_experiment(&project, &protocol, &events)?;
            if view.state != ExperimentRunState::Completed || view.final_decision.is_none() {
                anyhow::bail!(
                    "only a deeply replayed completed experiment can become a historical anchor"
                );
            }
            let sealed_report = view
                .sealed_report
                .as_ref()
                .context("completed experiment has no sealed report")?;
            let sealed_assessment = view
                .sealed_assessment
                .as_ref()
                .context("completed experiment has no sealed assessment")?;
            let history = HistoricalBenchmarkConsumption::create(
                project.id,
                project.fingerprint,
                view.run_id,
                protocol.fingerprint,
                sealed_report.suite_fingerprint.clone(),
                sealed_assessment.id,
                sealed_assessment.fingerprint.clone(),
                view.last_event_fingerprint,
                view.updated_at,
                args.recorded_by,
            )?;
            let generation = BenchmarkGeneration::historical_exhausted_anchor(history, Utc::now())?;
            let first = first_generation_event(&generation, generation.created_at)?;
            store
                .create_benchmark_generation(&generation, &first)
                .await?;
            print_generation(
                &generation,
                &replay_benchmark_generation(&generation, &[first])?,
            )
        }
        BenchmarkGenerationCommand::NomosCreate(args) => {
            let predecessor = match args.predecessor_generation_id {
                Some(id) => Some(store.get_benchmark_generation(id).await?.with_context(|| {
                    format!("predecessor benchmark generation {id} does not exist")
                })?),
                None => None,
            };
            let backend = NomosBackend::open(&args.backend.workspace, args.backend.python)?;
            let generation = backend.benchmark_generation(predecessor.as_ref())?;
            let first = first_generation_event(&generation, generation.created_at)?;
            store
                .create_benchmark_generation(&generation, &first)
                .await?;
            print_generation(
                &generation,
                &replay_benchmark_generation(&generation, &[first])?,
            )
        }
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
    let store = command.database_access().production(database_url).await?;
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
        ProductionCampaignCommand::Doctor(args) => {
            let provenance = campaign_provenance(&store, &backend, args.campaign_id).await?;
            presentation::print(&serde_json::json!({
                "verified": true,
                "campaign_id": args.campaign_id,
                "provenance_fingerprint": artifact_core::fingerprint(&provenance)?,
                "state": provenance["campaign"]["state"],
                "decision": provenance["experiment"]["final_decision"],
            }))
        }
        ProductionCampaignCommand::Provenance(args) => {
            presentation::print(&campaign_provenance(&store, &backend, args.campaign_id).await?)
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

pub(crate) struct CampaignContext {
    pub(crate) campaign: ProductionCampaign,
    pub(crate) view: CampaignView,
    pub(crate) generation: Option<(BenchmarkGeneration, BenchmarkGenerationView)>,
    pub(crate) experiment: Option<ExperimentView>,
}

pub(crate) async fn campaign_provenance(
    store: &SqliteExperimentStore,
    backend: &impl EncoderTaskBackend,
    campaign_id: Uuid,
) -> anyhow::Result<serde_json::Value> {
    let context = load_campaign_context(store, backend, campaign_id).await?;
    let project = store
        .get_project(context.campaign.project_snapshot_id)
        .await?
        .context("campaign project does not exist")?;
    let protocol = match context.view.protocol_id {
        Some(id) => Some(
            store
                .get_protocol(id)
                .await?
                .with_context(|| format!("campaign protocol {id} does not exist"))?,
        ),
        None => None,
    };
    if let Some(protocol) = &protocol {
        protocol.validate_integrity(&project)?;
        if Some(protocol.fingerprint.as_str()) != context.view.protocol_fingerprint.as_deref() {
            anyhow::bail!("campaign protocol binding changed");
        }
    }
    let predecessor = match context
        .generation
        .as_ref()
        .and_then(|value| value.0.predecessor_id)
    {
        Some(id) => {
            let (generation, view) = load_generation(store, id).await?;
            let current = &context
                .generation
                .as_ref()
                .expect("predecessor implies current generation")
                .0;
            if current.predecessor_fingerprint.as_deref() != Some(generation.fingerprint.as_str())
                || view.state != BenchmarkGenerationState::Superseded
            {
                anyhow::bail!("campaign generation predecessor chain changed");
            }
            Some((generation, view))
        }
        None => None,
    };
    let campaign_events = store.list_campaign_events(campaign_id).await?;
    let sealed_exposures = campaign_events
        .iter()
        .filter_map(|event| match &event.event {
            CampaignEventKind::IterationFinalized {
                sealed_exposure: Some(exposure),
                ..
            } => Some(exposure.as_ref()),
            _ => None,
        })
        .collect::<Vec<_>>();
    if sealed_exposures.len() > context.campaign.budget.maximum_sealed_evaluations as usize {
        anyhow::bail!("campaign persisted more sealed exposures than its finite budget");
    }
    if let Some((_, generation_view)) = &context.generation {
        if sealed_exposures.is_empty() && generation_view.consuming_experiment_run_id.is_some() {
            anyhow::bail!("generation claims consumption without a campaign sealed exposure");
        }
        if !sealed_exposures.is_empty()
            && !matches!(
                generation_view.state,
                BenchmarkGenerationState::Exhausted | BenchmarkGenerationState::Superseded
            )
        {
            anyhow::bail!("campaign sealed exposure did not exhaust its generation");
        }
    }
    let candidates = context
        .experiment
        .as_ref()
        .map(|experiment| {
            experiment
                .candidates
                .iter()
                .map(|(candidate_id, execution)| {
                    let suites = execution
                        .development_assessments
                        .iter()
                        .map(|(suite_key, assessment)| {
                            let report = execution
                                .development_reports
                                .get(suite_key)
                                .expect("deep replay requires one report per assessment");
                            serde_json::json!({
                                "suite_key": suite_key,
                                "report_id": report.id,
                                "report_fingerprint": report.fingerprint,
                                "assessment_id": assessment.id,
                                "assessment_fingerprint": assessment.fingerprint,
                                "verdict": assessment.verdict,
                                "primary_improvement": assessment.primary_improvement,
                            })
                        })
                        .collect::<Vec<_>>();
                    serde_json::json!({
                        "candidate_id": candidate_id,
                        "state": execution.state,
                        "model_fingerprint": execution
                            .train_output
                            .as_ref()
                            .map(|output| output.model.fingerprint.as_str()),
                        "development_suites": suites,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(serde_json::json!({
        "campaign": {
            "id": context.campaign.id,
            "fingerprint": context.campaign.fingerprint,
            "state": context.view.state,
            "event_count": campaign_events.len(),
            "head_fingerprint": context.view.last_event_fingerprint,
            "budget": context.campaign.budget,
            "reserved_usage": context.view.reserved_usage,
        },
        "project": {
            "id": project.id,
            "fingerprint": project.fingerprint,
            "source_revision": project.source_revision,
            "source_fingerprint": project.source_fingerprint,
        },
        "predecessor_generation": predecessor.as_ref().map(|(generation, view)| serde_json::json!({
            "id": generation.id,
            "fingerprint": generation.fingerprint,
            "state": view.state,
            "head_fingerprint": view.last_event_fingerprint,
            "historical_consumption": generation.historical_consumption,
        })),
        "benchmark_generation": context.generation.as_ref().map(|(generation, view)| serde_json::json!({
            "id": generation.id,
            "fingerprint": generation.fingerprint,
            "state": view.state,
            "adaptive_eligible": view.is_adaptive_eligible(),
            "head_fingerprint": view.last_event_fingerprint,
            "sealed_suite_fingerprint": generation.sealed_suite_fingerprint,
            "development_suite_keys": generation.development_suites.iter().map(|value| value.suite_key.as_str()).collect::<Vec<_>>(),
        })),
        "protocol": protocol.as_ref().map(|value| serde_json::json!({
            "id": value.id,
            "fingerprint": value.fingerprint,
            "development_suite_keys": value.development_suite_keys(),
            "sealed_suite_key": value.sealed_suite_key,
            "candidate_count": value.candidates.len(),
        })),
        "experiment": context.experiment.as_ref().map(|value| serde_json::json!({
            "run_id": value.run_id,
            "state": value.state,
            "head_fingerprint": value.last_event_fingerprint,
            "selected_candidate_id": value.selected_candidate_id,
            "final_decision": value.final_decision,
            "candidates": candidates,
        })),
        "sealed_exposures": sealed_exposures,
    }))
}

pub(crate) async fn load_campaign_context(
    store: &SqliteExperimentStore,
    backend: &impl EncoderTaskBackend,
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

pub(crate) async fn load_generation(
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
    generation
        .0
        .freshness
        .as_ref()
        .context("bound benchmark generation has no renewable freshness authority")?
        .validate_at(Utc::now())?;
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

pub(crate) async fn advance_campaign(
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

pub(crate) async fn finalize_campaign_iteration(
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
    let terminal = match &exposure {
        Some(exposure) => Some(
            generation_view.next_event(
                generation,
                BenchmarkGenerationEventKind::IterationConsumed {
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
                now,
            )?,
        ),
        None => None,
    };
    let campaign_event = finalize_iteration_event(
        &context.campaign,
        &context.view,
        experiment,
        terminal.as_ref(),
        exposure,
        now,
    )?;
    if let Some(terminal) = terminal {
        store
            .append_iteration_finalization(&terminal, &campaign_event)
            .await?;
    } else {
        store.append_campaign_event(&campaign_event).await?;
    }
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
        "fresh_at_command_time": generation
            .freshness
            .as_ref()
            .is_some_and(|freshness| freshness.validate_at(Utc::now()).is_ok()),
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
        BenchmarkGenerationCommand::NomosBuildAuthority(args) => &args.backend,
        BenchmarkGenerationCommand::MigrateConsumed(args) => &args.backend,
        BenchmarkGenerationCommand::NomosCreate(args) => &args.backend,
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
        | ProductionCampaignCommand::Doctor(ProductionCampaignIdArgs { backend, .. })
        | ProductionCampaignCommand::Provenance(ProductionCampaignIdArgs { backend, .. })
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
