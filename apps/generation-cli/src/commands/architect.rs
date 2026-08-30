use std::{path::PathBuf, sync::Arc, time::Duration};

use analysis_core::{
    contract::DiagnosticContract,
    ports::{AnalysisStore, FindingReviewQuery},
};
use anyhow::{Context, ensure};
use dataset_architect_core::{
    brief::{
        ARCHITECT_BRIEF_SCHEMA_VERSION, ArchitectBudgets, ArchitectProviderConfiguration,
        GenerationCostModel, GovernedDiagnosticContext, PlanningPriority, ResolvedArchitectBrief,
    },
    lifecycle::{
        ArchitectRun, ArchitectRunState, ArchitectStopReason, ArchitectToolCall,
        ArchitectToolCallState,
    },
    ports::ArchitectStore,
    proposal::{
        ArchitectProposalReview, ArchitectReviewDecision, DatasetArchitectureProposal,
        apply_approved_proposal,
    },
};
use dataset_architect_runner::ArchitectRunner;
use evaluation_core::ports::EvaluationStore;
use generation_core::{
    dimensions::expand_generation_cells,
    ports::{DatasetStore, RowStore},
};
use research_agent_pi_process::PiProcessRuntime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;
use workflow_core::{
    allocation::{InitialCellConstraint, InitialCellCoverage},
    governance::{
        CohortDisposition, CohortRole, DisclosureLevel, EvidenceExposure, EvidenceExposureRequest,
        ExposurePurpose,
    },
    ports::{CohortQuery, GovernanceStore},
};

use crate::{
    cli::{ArchitectCommand, ArchitectReviewArgs, ArchitectStartArgs, ResearchRuntimeArgs},
    presentation,
};

pub async fn execute(command: ArchitectCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        ArchitectCommand::BriefValidate { file } => {
            let (brief, exposure) = resolve_brief(store, &file).await?;
            if let Some(exposure) = exposure {
                store.append_exposure(&exposure, None).await?;
            }
            presentation::print(&brief)
        }
        ArchitectCommand::Start(args) => start(args, store).await,
        ArchitectCommand::Status { run_id } => presentation::print(&status(store, run_id).await?),
        ArchitectCommand::Watch { run_id } => watch(store, run_id).await,
        ArchitectCommand::Proposal { run_id } => {
            require_run(store, run_id).await?;
            presentation::print(
                &store
                    .latest_proposal_for_run(run_id)
                    .await?
                    .with_context(|| format!("architect run {run_id} has no proposal"))?,
            )
        }
        ArchitectCommand::Cancel { run_id } => cancel(store, run_id).await,
        ArchitectCommand::Recover { run_id } => recover(store, run_id).await,
        ArchitectCommand::Review(args) => review(store, args).await,
        ArchitectCommand::Apply { proposal_id } => apply(store, proposal_id).await,
        ArchitectCommand::Context { plan_id } => presentation::print(
            &store
                .generation_strategy_context_for_plan(plan_id)
                .await?
                .with_context(|| {
                    format!("generation plan {plan_id} has no approved strategy context")
                })?,
        ),
    }
}

async fn start(args: ArchitectStartArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let (brief, exposure) = resolve_brief(store, &args.file).await?;
    if let Some(exposure) = exposure {
        store.append_exposure(&exposure, None).await?;
    }
    let sidecar = resolve_sidecar(&args.runtime)?;
    let mut runtime = PiProcessRuntime::new(args.runtime.node, sidecar);
    if brief.provider.provider == "fake" {
        let script = args
            .script
            .context("--script is required for a fake architect provider")?;
        runtime = runtime.with_scripted_turns(read_json(&script)?);
    } else {
        ensure!(
            args.script.is_none(),
            "--script is only valid for a fake architect provider"
        );
    }
    let runner = ArchitectRunner::new(Arc::new(runtime), Arc::new(store.clone()));
    let run = runner.queue(brief.clone()).await?;
    eprintln!(
        "architect run {}: provider={}/{} external_calls={} limits: {} turns, {} tools, {} previews, {} seconds",
        run.id,
        brief.provider.provider,
        brief.provider.model,
        brief.provider.provider != "fake",
        brief.budgets.max_model_turns,
        brief.budgets.max_tool_calls,
        brief.budgets.max_allocation_previews,
        brief.budgets.max_wall_clock_seconds,
    );
    presentation::print(&runner.run(run.id).await?)
}

async fn cancel(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    let mut run = require_run(store, run_id).await?;
    match run.state {
        ArchitectRunState::Queued => run.cancel()?,
        ArchitectRunState::Running => run.request_cancel()?,
        _ => anyhow::bail!("cannot cancel a {:?} architect run", run.state),
    }
    store.save_run(&run).await?;
    presentation::print(&run)
}

async fn recover(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    let mut run = require_run(store, run_id).await?;
    if run.state == ArchitectRunState::Running {
        for mut call in store.list_tool_calls(run.id).await? {
            if call.state == ArchitectToolCallState::Started {
                call.interrupt()?;
                store.record_tool_call(&call).await?;
            }
        }
        run.fail(
            ArchitectStopReason::Interrupted,
            "architect host stopped; open tool calls were not replayed".into(),
        )?;
        store.save_run(&run).await?;
    }
    presentation::print(&run)
}

async fn review(store: &SqliteStore, args: ArchitectReviewArgs) -> anyhow::Result<()> {
    let proposal = store
        .get_proposal(args.proposal_id)
        .await?
        .with_context(|| format!("architecture proposal not found: {}", args.proposal_id))?;
    let run = require_run(store, proposal.run_id).await?;
    ensure!(
        run.state == ArchitectRunState::AwaitingReview,
        "proposal run is not awaiting review"
    );
    let predecessor = store.latest_review(proposal.id).await?;
    let decision = if args.approve {
        ArchitectReviewDecision::Approve
    } else if args.reject {
        ArchitectReviewDecision::Reject
    } else {
        ArchitectReviewDecision::RequestRevision
    };
    let review = ArchitectProposalReview::create(
        &proposal,
        predecessor.as_ref(),
        decision,
        args.reviewer,
        args.reason,
    )?;
    store.append_review(&review).await?;
    presentation::print(&review)
}

async fn apply(store: &SqliteStore, proposal_id: Uuid) -> anyhow::Result<()> {
    if let Some(existing) = store.get_application(proposal_id).await? {
        return presentation::print(&existing);
    }
    let proposal = store
        .get_proposal(proposal_id)
        .await?
        .with_context(|| format!("architecture proposal not found: {proposal_id}"))?;
    let review = store
        .latest_review(proposal.id)
        .await?
        .context("architecture proposal has no review")?;
    let brief = store
        .get_brief(proposal.brief_id)
        .await?
        .context("architecture brief is missing")?;
    let coverage = current_coverage(store, &brief.dataset).await?;
    let applied = apply_approved_proposal(&proposal, &brief, &review, &coverage)?;
    store
        .save_application(
            &applied.application,
            &applied.plan,
            &applied.strategy_context,
        )
        .await?;
    presentation::print(&applied)
}

async fn watch(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    loop {
        let view = status(store, run_id).await?;
        eprintln!(
            "architect {}: {:?}, turns={}, tools={}, previews={}",
            run_id,
            view.run.state,
            view.run.usage.model_turns,
            view.run.usage.tool_calls,
            view.run.usage.allocation_previews
        );
        if !matches!(
            view.run.state,
            ArchitectRunState::Queued | ArchitectRunState::Running
        ) {
            return presentation::print(&view);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[derive(Debug, Serialize)]
struct ArchitectStatus {
    run: ArchitectRun,
    brief: ResolvedArchitectBrief,
    tool_calls: Vec<ArchitectToolCall>,
    proposal: Option<DatasetArchitectureProposal>,
    latest_review: Option<ArchitectProposalReview>,
}

async fn status(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<ArchitectStatus> {
    let run = require_run(store, run_id).await?;
    let brief = store
        .get_brief(run.brief_id)
        .await?
        .context("architect brief is missing")?;
    let proposal = store.latest_proposal_for_run(run_id).await?;
    let latest_review = match &proposal {
        Some(value) => store.latest_review(value.id).await?,
        None => None,
    };
    Ok(ArchitectStatus {
        run,
        brief,
        tool_calls: store.list_tool_calls(run_id).await?,
        proposal,
        latest_review,
    })
}

async fn require_run(store: &SqliteStore, id: Uuid) -> anyhow::Result<ArchitectRun> {
    store
        .get_run(id)
        .await?
        .with_context(|| format!("architect run not found: {id}"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchitectBriefFile {
    schema_version: u32,
    #[serde(default)]
    dataset_id: Option<Uuid>,
    #[serde(default)]
    dataset_name: Option<String>,
    target_total_rows: u32,
    #[serde(default)]
    reserved_rows: u32,
    #[serde(default)]
    constraints: Vec<InitialCellConstraint>,
    priorities: Vec<PlanningPriority>,
    #[serde(default = "yes")]
    use_semantic_context: bool,
    #[serde(default = "yes")]
    use_authenticity_context: bool,
    #[serde(default)]
    analysis_report_id: Option<Uuid>,
    cost_model: GenerationCostModel,
    budgets: ArchitectBudgets,
    provider: ArchitectProviderConfiguration,
}

async fn resolve_brief(
    store: &SqliteStore,
    path: &PathBuf,
) -> anyhow::Result<(ResolvedArchitectBrief, Option<EvidenceExposure>)> {
    let input: ArchitectBriefFile = read_document(path)?;
    ensure!(
        input.schema_version == ARCHITECT_BRIEF_SCHEMA_VERSION,
        "unsupported architect brief schema version {}",
        input.schema_version
    );
    ensure!(
        input.dataset_id.is_some() ^ input.dataset_name.is_some(),
        "architect brief must specify exactly one of dataset_id or dataset_name"
    );
    let dataset = if let Some(id) = input.dataset_id {
        store
            .get_dataset(id)
            .await?
            .with_context(|| format!("dataset not found: {id}"))?
    } else {
        let name = input.dataset_name.as_deref().unwrap_or_default();
        let mut matches = store
            .list_datasets()
            .await?
            .into_iter()
            .filter(|value| value.name == name);
        let dataset = matches
            .next()
            .with_context(|| format!("dataset name {name:?} was not found"))?;
        ensure!(
            matches.next().is_none(),
            "dataset name {name:?} must resolve uniquely"
        );
        dataset
    };
    let coverage = current_coverage(store, &dataset).await?;
    let semantics = if input.use_semantic_context {
        Some(super::semantic::resolve_dataset_semantics(store, dataset.id).await?)
    } else {
        None
    };
    let authenticity = if input.use_authenticity_context {
        research_core::ports::ResearchStore::resolve_context(store, dataset.id).await?
    } else {
        None
    };
    let (development_evidence, exposure) = match input.analysis_report_id {
        Some(id) => {
            let (context, exposure) = resolve_development_evidence(store, id).await?;
            (Some(context), Some(exposure))
        }
        None => (None, None),
    };
    let brief = ResolvedArchitectBrief::create(
        dataset,
        input.target_total_rows,
        input.reserved_rows,
        coverage,
        input.constraints,
        input.priorities,
        semantics,
        authenticity,
        development_evidence,
        input.cost_model,
        input.budgets,
        input.provider,
    )?;
    Ok((brief, exposure))
}

async fn resolve_development_evidence(
    store: &SqliteStore,
    report_id: Uuid,
) -> anyhow::Result<(GovernedDiagnosticContext, EvidenceExposure)> {
    let report = store
        .get_analysis_report(report_id)
        .await?
        .with_context(|| format!("analysis report not found: {report_id}"))?;
    let reviews = store
        .query_finding_reviews(FindingReviewQuery {
            analysis_report_id: Some(report.id),
            state: None,
            limit: 10_000,
            offset: 0,
        })
        .await?;
    let diagnostic = DiagnosticContract::from_report_and_reviews(&report, &reviews)?;
    diagnostic.validate(true)?;
    let evaluation = store
        .get_evaluation_run(report.evaluation_run_id)
        .await?
        .context("analysis evaluation run is missing")?;
    let cohorts = store
        .query_cohorts(CohortQuery {
            snapshot_id: Some(evaluation.snapshot_id),
            limit: 10_000,
            offset: 0,
        })
        .await?;
    let mut eligible = Vec::new();
    for cohort in cohorts.into_iter().filter(|value| {
        value.split == evaluation.protocol.split
            && value.fingerprint == diagnostic.source_identity.cohort_fingerprint
    }) {
        let Some(role) = store.get_current_cohort_role(cohort.id).await? else {
            continue;
        };
        if role.disposition == CohortDisposition::Active
            && matches!(role.role, CohortRole::Development | CohortRole::Diagnostic)
        {
            eligible.push((cohort, role));
        }
    }
    ensure!(
        eligible.len() == 1,
        "analysis report must resolve to exactly one active development or diagnostic cohort"
    );
    let (cohort, role) = eligible.pop().expect("one eligible cohort checked");
    let exposure = EvidenceExposure::new(
        &cohort,
        &role,
        EvidenceExposureRequest {
            evaluation_run_id: Some(evaluation.id),
            workflow_run_id: None,
            workflow_iteration: None,
            purpose: ExposurePurpose::DatasetArchitecture,
            disclosure: DisclosureLevel::Slices,
            adaptation_eligible: true,
            note: Some(format!(
                "Dataset Architect consumed aggregate diagnostics from analysis report {}",
                report.id
            )),
        },
    )?;
    Ok((
        GovernedDiagnosticContext {
            cohort_id: cohort.id,
            cohort_fingerprint: cohort.fingerprint,
            role_decision_id: role.id,
            role_decision_fingerprint: role.fingerprint,
            role: role.role,
            disposition: role.disposition,
            diagnostic,
        },
        exposure,
    ))
}

async fn current_coverage(
    store: &SqliteStore,
    dataset: &generation_core::domain::DatasetDefinition,
) -> anyhow::Result<Vec<InitialCellCoverage>> {
    let counts = store.dataset_cell_counts(dataset.id).await?;
    Ok(expand_generation_cells(dataset)
        .into_iter()
        .map(|cell| InitialCellCoverage {
            accepted: counts.get(&cell.key()).map_or(0, |value| value.accepted),
            cell,
        })
        .collect())
}

fn resolve_sidecar(args: &ResearchRuntimeArgs) -> anyhow::Result<PathBuf> {
    let path = args.pi_sidecar.clone().unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../adapters/research-agent-pi/dist/main.js")
    });
    ensure!(
        path.is_file(),
        "Pi sidecar is missing at {}; run `npm ci && npm run build` in adapters/research-agent-pi or pass --pi-sidecar",
        path.display()
    );
    Ok(path)
}

fn read_json(path: &PathBuf) -> anyhow::Result<Value> {
    serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?,
    )
    .with_context(|| format!("invalid JSON in {}", path.display()))
}

fn read_document<T: for<'de> Deserialize<'de>>(path: &PathBuf) -> anyhow::Result<T> {
    let bytes =
        std::fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    match path.extension().and_then(|value| value.to_str()) {
        Some("toml") => toml::from_str(std::str::from_utf8(&bytes)?).map_err(Into::into),
        _ => serde_json::from_slice(&bytes).map_err(Into::into),
    }
}

const fn yes() -> bool {
    true
}
