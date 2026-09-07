use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, ensure};
use benchmark_architect_core::{
    blueprint::{
        BenchmarkAcquisitionHandoff, BenchmarkArchitectureProposal, BenchmarkArchitectureReview,
        BenchmarkArchitectureReviewDecision, CandidateCohortFacts, assess_acquisition_conformance,
    },
    brief::{
        BENCHMARK_ARCHITECT_BRIEF_SCHEMA_VERSION, BenchmarkArchitectBriefDraft,
        ResolvedBenchmarkArchitectBrief,
    },
    lifecycle::{
        BenchmarkArchitectRun, BenchmarkArchitectRunState, BenchmarkArchitectStopReason,
        BenchmarkArchitectToolCall, BenchmarkArchitectToolCallState,
    },
    ports::{BenchmarkArchitectStore, PageFetcher, SearchProvider},
};
use benchmark_architect_runner::BenchmarkArchitectRunner;
use chrono::Utc;
use research_agent_pi_process::PiProcessRuntime;
use research_web::{BraveSearchProvider, CorpusWebAdapter, HttpPageFetcher};
use serde::{Deserialize, Serialize};
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::{
    cli::{BenchmarkArchitectCommand, BenchmarkArchitectReviewArgs, BenchmarkArchitectStartArgs},
    document, presentation,
};

const ACQUISITION_FACTS_SCHEMA_VERSION: u32 = 1;

pub async fn execute(
    command: BenchmarkArchitectCommand,
    store: &SqliteStore,
) -> anyhow::Result<()> {
    match command {
        BenchmarkArchitectCommand::BriefValidate { file } => {
            presentation::print(&resolve_brief(&file)?)
        }
        BenchmarkArchitectCommand::Start(args) => start(args, store).await,
        BenchmarkArchitectCommand::Status { run_id } => {
            presentation::print(&status(store, run_id).await?)
        }
        BenchmarkArchitectCommand::Watch { run_id } => watch(store, run_id).await,
        BenchmarkArchitectCommand::Evidence { run_id } => {
            require_run(store, run_id).await?;
            presentation::print(&BenchmarkArchitectStore::list_evidence(store, run_id).await?)
        }
        BenchmarkArchitectCommand::Proposal { run_id } => {
            require_run(store, run_id).await?;
            presentation::print(
                &BenchmarkArchitectStore::latest_proposal_for_run(store, run_id)
                    .await?
                    .with_context(|| format!("benchmark architect run {run_id} has no proposal"))?,
            )
        }
        BenchmarkArchitectCommand::Cancel { run_id } => cancel(store, run_id).await,
        BenchmarkArchitectCommand::Recover { run_id } => recover(store, run_id).await,
        BenchmarkArchitectCommand::Review(args) => review(store, args).await,
        BenchmarkArchitectCommand::Handoff { proposal_id } => handoff(store, proposal_id).await,
        BenchmarkArchitectCommand::HandoffShow { id } => presentation::print(
            &BenchmarkArchitectStore::get_handoff_by_id(store, id)
                .await?
                .with_context(|| format!("benchmark acquisition handoff not found: {id}"))?,
        ),
        BenchmarkArchitectCommand::Conformance { handoff_id, file } => {
            conformance(store, handoff_id, &file).await
        }
    }
}

async fn start(args: BenchmarkArchitectStartArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let brief = resolve_brief(&args.file)?;
    ensure!(
        brief.provider.runtime == "pi",
        "Benchmark Architect currently requires provider.runtime = \"pi\""
    );
    let sidecar = super::research::resolve_sidecar(&args.runtime)?;
    let mut runtime = PiProcessRuntime::new(args.runtime.node, sidecar);
    let (search, fetcher): (Arc<dyn SearchProvider>, Arc<dyn PageFetcher>) =
        if brief.provider.provider == "fake" {
            let script = args
                .script
                .context("--script is required for a fake Benchmark Architect provider")?;
            let corpus = args
                .corpus
                .context("--corpus is required for a fake Benchmark Architect provider")?;
            runtime = runtime.with_scripted_turns(super::research::read_json(&script)?);
            let corpus = Arc::new(CorpusWebAdapter::from_json(
                &std::fs::read(&corpus)
                    .with_context(|| format!("could not read corpus {}", corpus.display()))?,
            )?);
            (corpus.clone(), corpus)
        } else {
            ensure!(
                args.script.is_none() && args.corpus.is_none(),
                "--script and --corpus are only valid for a fake Benchmark Architect provider"
            );
            let search_api_key = std::env::var(&args.search_api_key_env).with_context(|| {
                format!(
                    "search API key environment variable {} is not set",
                    args.search_api_key_env
                )
            })?;
            (
                Arc::new(BraveSearchProvider::new(search_api_key)?),
                Arc::new(HttpPageFetcher::new(brief.source_policy.clone())?),
            )
        };
    let runner =
        BenchmarkArchitectRunner::new(Arc::new(runtime), search, fetcher, Arc::new(store.clone()));
    let run = runner.queue(brief.clone()).await?;
    eprintln!(
        "benchmark architect run {}: provider={}/{} external_calls={} limits: {} turns, {} tools, {} searches, {} pages, {} previews, {} seconds",
        run.id,
        brief.provider.provider,
        brief.provider.model,
        brief.provider.provider != "fake",
        brief.budgets.max_model_turns,
        brief.budgets.max_tool_calls,
        brief.budgets.max_searches,
        brief.budgets.max_fetched_pages,
        brief.budgets.max_blueprint_previews,
        brief.budgets.max_wall_clock_seconds,
    );
    presentation::print(&runner.run(run.id).await?)
}

async fn cancel(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    let mut run = require_run(store, run_id).await?;
    match run.state {
        BenchmarkArchitectRunState::Queued => run.cancel()?,
        BenchmarkArchitectRunState::Running => run.request_cancel()?,
        _ => anyhow::bail!("cannot cancel a {:?} benchmark architect run", run.state),
    }
    BenchmarkArchitectStore::save_run(store, &run).await?;
    presentation::print(&run)
}

async fn recover(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    let mut run = require_run(store, run_id).await?;
    if run.state == BenchmarkArchitectRunState::Running {
        for mut call in BenchmarkArchitectStore::list_tool_calls(store, run.id).await? {
            if call.state == BenchmarkArchitectToolCallState::Started {
                call.interrupt()?;
                BenchmarkArchitectStore::record_tool_call(store, &call).await?;
            }
        }
        run.fail(
            BenchmarkArchitectStopReason::Interrupted,
            "benchmark architect host stopped; open tool calls were not replayed".into(),
        )?;
        BenchmarkArchitectStore::save_run(store, &run).await?;
    }
    presentation::print(&run)
}

async fn review(store: &SqliteStore, args: BenchmarkArchitectReviewArgs) -> anyhow::Result<()> {
    let proposal = BenchmarkArchitectStore::get_proposal(store, args.proposal_id)
        .await?
        .with_context(|| {
            format!(
                "benchmark architecture proposal not found: {}",
                args.proposal_id
            )
        })?;
    let run = require_run(store, proposal.run_id).await?;
    ensure!(
        run.state == BenchmarkArchitectRunState::AwaitingReview,
        "proposal run is not awaiting review"
    );
    let predecessor = BenchmarkArchitectStore::latest_review(store, proposal.id).await?;
    let decision = if args.approve {
        BenchmarkArchitectureReviewDecision::Approve
    } else if args.reject {
        BenchmarkArchitectureReviewDecision::Reject
    } else {
        BenchmarkArchitectureReviewDecision::RequestRevision
    };
    let review = BenchmarkArchitectureReview::create(
        &proposal,
        predecessor.as_ref(),
        decision,
        args.reviewer,
        args.reason,
    )?;
    BenchmarkArchitectStore::append_review(store, &review).await?;
    presentation::print(&review)
}

async fn handoff(store: &SqliteStore, proposal_id: Uuid) -> anyhow::Result<()> {
    if let Some(existing) = BenchmarkArchitectStore::get_handoff(store, proposal_id).await? {
        return presentation::print(&existing);
    }
    let proposal = BenchmarkArchitectStore::get_proposal(store, proposal_id)
        .await?
        .with_context(|| format!("benchmark architecture proposal not found: {proposal_id}"))?;
    let brief = BenchmarkArchitectStore::get_brief(store, proposal.brief_id)
        .await?
        .context("benchmark architect brief is missing")?;
    let approval = BenchmarkArchitectStore::latest_review(store, proposal.id)
        .await?
        .context("benchmark architecture proposal has no review")?;
    let evidence = BenchmarkArchitectStore::list_evidence(store, proposal.run_id).await?;
    let handoff = BenchmarkAcquisitionHandoff::create(&brief, &proposal, &approval, &evidence)?;
    BenchmarkArchitectStore::save_handoff(store, &handoff).await?;
    presentation::print(&handoff)
}

async fn conformance(store: &SqliteStore, handoff_id: Uuid, file: &Path) -> anyhow::Result<()> {
    let handoff = BenchmarkArchitectStore::get_handoff_by_id(store, handoff_id)
        .await?
        .with_context(|| format!("benchmark acquisition handoff not found: {handoff_id}"))?;
    let input: AcquisitionFactsFile = document::read(file)?;
    ensure!(
        input.schema_version == ACQUISITION_FACTS_SCHEMA_VERSION,
        "unsupported acquisition facts schema version {}",
        input.schema_version
    );
    presentation::print(&assess_acquisition_conformance(
        &handoff,
        &input.candidates,
        Utc::now(),
    )?)
}

async fn watch(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    loop {
        let view = status(store, run_id).await?;
        eprintln!(
            "benchmark architect {}: {:?}, turns={}, tools={}, searches={}, pages={}, evidence={}, previews={}",
            run_id,
            view.run.state,
            view.run.usage.model_turns,
            view.run.usage.tool_calls,
            view.run.usage.searches,
            view.run.usage.fetched_pages,
            view.evidence_count,
            view.run.usage.blueprint_previews,
        );
        if !matches!(
            view.run.state,
            BenchmarkArchitectRunState::Queued | BenchmarkArchitectRunState::Running
        ) {
            return presentation::print(&view);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[derive(Debug, Serialize)]
struct BenchmarkArchitectStatus {
    run: BenchmarkArchitectRun,
    brief: ResolvedBenchmarkArchitectBrief,
    tool_calls: Vec<BenchmarkArchitectToolCall>,
    evidence_count: usize,
    proposal: Option<BenchmarkArchitectureProposal>,
    latest_review: Option<BenchmarkArchitectureReview>,
    handoff: Option<BenchmarkAcquisitionHandoff>,
}

async fn status(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<BenchmarkArchitectStatus> {
    let run = require_run(store, run_id).await?;
    let brief = BenchmarkArchitectStore::get_brief(store, run.brief_id)
        .await?
        .context("benchmark architect brief is missing")?;
    let proposal = BenchmarkArchitectStore::latest_proposal_for_run(store, run_id).await?;
    let latest_review = match &proposal {
        Some(value) => BenchmarkArchitectStore::latest_review(store, value.id).await?,
        None => None,
    };
    let handoff = match &proposal {
        Some(value) => BenchmarkArchitectStore::get_handoff(store, value.id).await?,
        None => None,
    };
    let evidence = BenchmarkArchitectStore::list_evidence(store, run_id).await?;
    Ok(BenchmarkArchitectStatus {
        run,
        brief,
        tool_calls: BenchmarkArchitectStore::list_tool_calls(store, run_id).await?,
        evidence_count: evidence.len(),
        proposal,
        latest_review,
        handoff,
    })
}

async fn require_run(store: &SqliteStore, id: Uuid) -> anyhow::Result<BenchmarkArchitectRun> {
    BenchmarkArchitectStore::get_run(store, id)
        .await?
        .with_context(|| format!("benchmark architect run not found: {id}"))
}

#[derive(Debug, Deserialize)]
struct BenchmarkArchitectBriefFile {
    schema_version: u32,
    #[serde(flatten)]
    draft: BenchmarkArchitectBriefDraft,
}

pub(super) fn resolve_brief(path: &Path) -> anyhow::Result<ResolvedBenchmarkArchitectBrief> {
    let input: BenchmarkArchitectBriefFile = document::read(path)?;
    ensure!(
        input.schema_version == BENCHMARK_ARCHITECT_BRIEF_SCHEMA_VERSION,
        "unsupported benchmark architect brief schema version {}",
        input.schema_version
    );
    ResolvedBenchmarkArchitectBrief::create(input.draft).map_err(Into::into)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AcquisitionFactsFile {
    schema_version: u32,
    candidates: Vec<CandidateCohortFacts>,
}
