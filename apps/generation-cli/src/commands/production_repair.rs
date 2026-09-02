use std::collections::BTreeMap;

use anyhow::Context;
use chrono::Utc;
use encoder_campaign_core::{CampaignEventKind, CampaignStore, replay_campaign};
use encoder_experiment_core::{
    metrics::EvaluationReport,
    ports::{EncoderTaskBackend, ExperimentStore},
};
use encoder_experiment_nomos::NomosBackend;
use encoder_experiment_runner::ExperimentRunner;
use encoder_experiment_sqlite::SqliteExperimentStore;
use encoder_repair_core::{
    collection::DevelopmentObservationRequest,
    diagnosis::{CandidateSuiteOutcome, ComparativeDiagnosis},
    observation::DevelopmentObservationSet,
    ports::{DevelopmentObservationBackend, RepairEvidenceStore},
};
use uuid::Uuid;

use crate::{
    cli::{
        NomosWorkspaceArgs, ProductionRepairCampaignArgs, ProductionRepairCommand,
        ProductionRepairDiagnoseArgs, ProductionRepairIdArgs,
    },
    commands::experiment::ensure_database_belongs_to_workspace,
    presentation,
};

pub async fn execute(command: ProductionRepairCommand, database_url: &str) -> anyhow::Result<()> {
    let backend_args = backend_args(&command);
    ensure_database_belongs_to_workspace(database_url, &backend_args.workspace)?;
    let store = SqliteExperimentStore::connect(database_url).await?;
    let backend = NomosBackend::open(&backend_args.workspace, backend_args.python.clone())?;

    match command {
        ProductionRepairCommand::Diagnose(args) => diagnose(&store, &backend, args).await,
        ProductionRepairCommand::Show(args) => show(&store, &backend, args).await,
        ProductionRepairCommand::Doctor(args) => doctor(&store, &backend, args).await,
        ProductionRepairCommand::Evidence(args) => evidence(&store, &backend, args).await,
    }
}

async fn diagnose(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairDiagnoseArgs,
) -> anyhow::Result<()> {
    if args.minimum_support == 0 || args.maximum_seconds == 0 {
        anyhow::bail!("repair support and collection time limits must be positive");
    }
    let campaign = store
        .get_campaign(args.campaign_id)
        .await?
        .with_context(|| format!("production campaign {} does not exist", args.campaign_id))?;
    let campaign_events = store.list_campaign_events(campaign.id).await?;
    replay_campaign(&campaign, &campaign_events)?;
    let linked_runs = campaign_events
        .iter()
        .filter_map(|event| match event.event {
            CampaignEventKind::RunStarted { run_id, .. } => Some(run_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let run_id = match args.run_id {
        Some(run_id) if linked_runs.contains(&run_id) => run_id,
        Some(_) => anyhow::bail!("requested repair run is not linked to this campaign"),
        None => *linked_runs
            .last()
            .context("campaign has no linked experiment run to diagnose")?,
    };
    let project = store
        .get_project(campaign.project_snapshot_id)
        .await?
        .context("repair campaign project does not exist")?;
    if project.fingerprint != campaign.project_snapshot_fingerprint {
        anyhow::bail!("repair campaign project fingerprint changed");
    }
    backend.inspect(project.clone()).await?;
    let experiment = ExperimentRunner::new(store, backend).status(run_id).await?;
    let protocol = store
        .get_protocol(experiment.protocol_id)
        .await?
        .context("repair source protocol does not exist")?;
    protocol.validate_integrity(&project)?;

    let baseline_reports = protocol
        .baseline_development_reports()
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    if baseline_reports.len() < 2 {
        anyhow::bail!("comparative repair diagnosis requires at least two development suites");
    }
    let baseline_by_suite = baseline_reports
        .iter()
        .map(|report| (report.suite_key.as_str(), report))
        .collect::<BTreeMap<_, _>>();
    let mut baseline_sets = Vec::with_capacity(baseline_reports.len());
    for report in &baseline_reports {
        baseline_sets.push(
            collect_and_persist(
                store,
                backend,
                &project,
                campaign.id,
                run_id,
                None,
                report,
                &protocol.metric_contract,
                args.dimensions.clone(),
                args.maximum_seconds,
            )
            .await?,
        );
    }

    let mut candidate_sets = Vec::new();
    let mut outcomes = Vec::new();
    for (candidate_id, execution) in &experiment.candidates {
        let train_output = execution.train_output.as_ref().with_context(|| {
            format!("repair candidate {candidate_id} has no immutable trained model")
        })?;
        if execution.development_reports.len() != baseline_by_suite.len()
            || execution.development_assessments.len() != baseline_by_suite.len()
        {
            anyhow::bail!(
                "repair candidate {candidate_id} has incomplete development-suite evidence"
            );
        }
        for (suite_key, baseline) in &baseline_by_suite {
            let report = execution
                .development_reports
                .get(*suite_key)
                .with_context(|| {
                    format!("repair candidate {candidate_id} omitted development suite {suite_key}")
                })?;
            let assessment = execution
                .development_assessments
                .get(*suite_key)
                .with_context(|| {
                    format!("repair candidate {candidate_id} omitted assessment for {suite_key}")
                })?;
            if report.model.fingerprint != train_output.model.fingerprint {
                anyhow::bail!("repair candidate report model identity changed");
            }
            outcomes.push(CandidateSuiteOutcome::create(
                &project,
                &protocol.metric_contract,
                *candidate_id,
                baseline,
                report,
                assessment,
            )?);
            candidate_sets.push(
                collect_and_persist(
                    store,
                    backend,
                    &project,
                    campaign.id,
                    run_id,
                    Some(*candidate_id),
                    report,
                    &protocol.metric_contract,
                    args.dimensions.clone(),
                    args.maximum_seconds,
                )
                .await?,
            );
        }
    }
    let diagnosis = ComparativeDiagnosis::create(
        &project,
        campaign.id,
        run_id,
        args.minimum_support,
        args.dimensions,
        &baseline_sets,
        &candidate_sets,
        &outcomes,
        Utc::now(),
    )?;
    let diagnosis = store.create_diagnosis(diagnosis).await?;
    print_diagnosis_summary(&diagnosis, baseline_sets.len(), candidate_sets.len())
}

#[allow(clippy::too_many_arguments)]
async fn collect_and_persist(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    project: &encoder_experiment_core::domain::ExternalProjectSnapshot,
    campaign_id: Uuid,
    run_id: Uuid,
    candidate_id: Option<Uuid>,
    report: &EvaluationReport,
    contract: &encoder_experiment_core::metrics::MetricContract,
    dimensions: Vec<String>,
    maximum_seconds: u64,
) -> anyhow::Result<DevelopmentObservationSet> {
    let request = DevelopmentObservationRequest::create(
        project,
        report,
        contract,
        dimensions,
        maximum_seconds,
    )?;
    let collected = backend
        .collect_development_observations(project.clone(), request.clone())
        .await?;
    collected.validate_for_request(&request)?;
    let set = DevelopmentObservationSet::create(
        project,
        campaign_id,
        run_id,
        candidate_id,
        report,
        contract,
        collected.observer,
        collected.observation_artifact,
        collected.observations,
        Utc::now(),
    )?;
    store.create_observation_set(set).await.map_err(Into::into)
}

async fn show(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairIdArgs,
) -> anyhow::Result<()> {
    let diagnosis = load_verified_diagnosis(store, backend, args.diagnosis_id).await?;
    presentation::print(&diagnosis)
}

async fn doctor(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairIdArgs,
) -> anyhow::Result<()> {
    let diagnosis = load_verified_diagnosis(store, backend, args.diagnosis_id).await?;
    presentation::print(&serde_json::json!({
        "verified": true,
        "diagnosis_id": diagnosis.id,
        "diagnosis_fingerprint": diagnosis.fingerprint,
        "derivation_fingerprint": diagnosis.derivation_fingerprint,
        "campaign_id": diagnosis.source_campaign_id,
        "experiment_run_id": diagnosis.source_experiment_run_id,
        "observation_set_count": diagnosis.observation_sets.len(),
        "sealed_observation_count": 0,
    }))
}

async fn evidence(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    args: ProductionRepairCampaignArgs,
) -> anyhow::Result<()> {
    let campaign = store
        .get_campaign(args.campaign_id)
        .await?
        .with_context(|| format!("production campaign {} does not exist", args.campaign_id))?;
    let project = store
        .get_project(campaign.project_snapshot_id)
        .await?
        .context("repair campaign project does not exist")?;
    backend.inspect(project).await?;
    let observations = store
        .list_observation_sets_for_campaign(campaign.id)
        .await?;
    let diagnoses = store.list_diagnoses_for_campaign(campaign.id).await?;
    let observation_summaries = observations
        .iter()
        .map(|value| {
            serde_json::json!({
                "id": value.id,
                "evidence_fingerprint": value.evidence_fingerprint,
                "fingerprint": value.fingerprint,
                "experiment_run_id": value.source_experiment_run_id,
                "candidate_id": value.candidate_id,
                "suite_key": value.suite_key,
                "model_fingerprint": value.model_fingerprint,
                "row_count": value.observations.len(),
                "artifact": value.observation_artifact,
            })
        })
        .collect::<Vec<_>>();
    let diagnosis_summaries = diagnoses
        .iter()
        .map(|value| {
            serde_json::json!({
                "id": value.id,
                "derivation_fingerprint": value.derivation_fingerprint,
                "fingerprint": value.fingerprint,
                "experiment_run_id": value.source_experiment_run_id,
                "minimum_support": value.minimum_support,
                "dimensions": value.slice_dimensions,
                "weakness_count": value.weaknesses.len(),
                "candidate_comparison_count": value.candidate_comparisons.len(),
            })
        })
        .collect::<Vec<_>>();
    presentation::print(&serde_json::json!({
        "campaign_id": campaign.id,
        "observation_sets": observation_summaries,
        "diagnoses": diagnosis_summaries,
    }))
}

async fn load_verified_diagnosis(
    store: &SqliteExperimentStore,
    backend: &NomosBackend,
    diagnosis_id: Uuid,
) -> anyhow::Result<ComparativeDiagnosis> {
    let diagnosis = store
        .get_diagnosis(diagnosis_id)
        .await?
        .with_context(|| format!("production repair diagnosis {diagnosis_id} does not exist"))?;
    let project = store
        .get_project(diagnosis.project_snapshot_id)
        .await?
        .context("repair diagnosis project does not exist")?;
    backend.inspect(project).await?;
    Ok(diagnosis)
}

fn print_diagnosis_summary(
    diagnosis: &ComparativeDiagnosis,
    baseline_set_count: usize,
    candidate_set_count: usize,
) -> anyhow::Result<()> {
    let tradeoffs = diagnosis
        .candidate_tradeoffs
        .iter()
        .map(|value| {
            serde_json::json!({
                "candidate_id": value.candidate_id,
                "model_fingerprint": value.model_fingerprint,
                "passed_suites": value.passed_suites,
                "failed_suites": value.failed_suites,
            })
        })
        .collect::<Vec<_>>();
    presentation::print(&serde_json::json!({
        "diagnosis_id": diagnosis.id,
        "fingerprint": diagnosis.fingerprint,
        "derivation_fingerprint": diagnosis.derivation_fingerprint,
        "campaign_id": diagnosis.source_campaign_id,
        "experiment_run_id": diagnosis.source_experiment_run_id,
        "baseline_observation_sets": baseline_set_count,
        "candidate_observation_sets": candidate_set_count,
        "weaknesses": diagnosis.weaknesses.len(),
        "candidate_comparisons": diagnosis.candidate_comparisons.len(),
        "candidate_tradeoffs": tradeoffs,
        "sealed_observation_count": 0,
    }))
}

fn backend_args(command: &ProductionRepairCommand) -> &NomosWorkspaceArgs {
    match command {
        ProductionRepairCommand::Diagnose(args) => &args.backend,
        ProductionRepairCommand::Show(args) | ProductionRepairCommand::Doctor(args) => {
            &args.backend
        }
        ProductionRepairCommand::Evidence(args) => &args.backend,
    }
}
