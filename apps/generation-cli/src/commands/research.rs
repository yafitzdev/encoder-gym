use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, ensure};
use generation_core::ports::{DatasetStore, PlanStore};
use research_agent_pi_process::PiProcessRuntime;
use research_core::{
    brief::{
        ArtifactReference, ResearchBriefDraft, ResearchBudgets, ResearchProviderConfiguration,
        ResearchTarget, ResolvedResearchBrief, SourcePolicy,
    },
    lifecycle::{
        ResearchRun, ResearchRunState, ResearchStopReason, ResearchToolCall, ToolCallState,
    },
    ports::{PageFetcher, ResearchStore, SearchProvider},
    profile::{ProfileBinding, ProfileReview, ProfileReviewDecision},
};
use research_runner::{RESEARCH_PROTOCOL_VERSION, ResearchRunner, protocol_fingerprint};
use research_web::{BraveSearchProvider, CorpusWebAdapter, HttpPageFetcher};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use synthetic_data_sqlite::SqliteStore;
use uuid::Uuid;

use crate::{
    cli::{ResearchCommand, ResearchReviewArgs, ResearchRuntimeArgs, ResearchStartArgs},
    presentation,
};

pub async fn execute(command: ResearchCommand, store: &SqliteStore) -> anyhow::Result<()> {
    match command {
        ResearchCommand::BriefValidate { file } => {
            presentation::print(&resolve_brief(store, &file).await?)
        }
        ResearchCommand::Start(args) => start(args, store).await,
        ResearchCommand::Status { run_id } => presentation::print(&status(store, run_id).await?),
        ResearchCommand::Watch { run_id } => watch(store, run_id).await,
        ResearchCommand::Evidence { run_id } => {
            require_run(store, run_id).await?;
            presentation::print(&store.list_evidence(run_id).await?)
        }
        ResearchCommand::Profile { run_id } => {
            require_run(store, run_id).await?;
            let profile = store
                .latest_profile_for_run(run_id)
                .await?
                .with_context(|| format!("research run {run_id} has no profile"))?;
            presentation::print(&profile)
        }
        ResearchCommand::Cancel { run_id } => {
            let mut run = require_run(store, run_id).await?;
            match run.state {
                ResearchRunState::Queued => run.cancel()?,
                ResearchRunState::Running => run.request_cancel()?,
                _ => anyhow::bail!("cannot cancel a {:?} research run", run.state),
            }
            store.save_run(&run).await?;
            presentation::print(&run)
        }
        ResearchCommand::Recover { run_id } => recover(run_id, store).await,
        ResearchCommand::Review(args) => review(args, store).await,
        ResearchCommand::Bind {
            dataset_or_plan_id,
            profile_id,
        } => bind(store, dataset_or_plan_id, profile_id).await,
        ResearchCommand::Context { dataset_or_plan_id } => {
            let dataset_id = resolve_dataset_id(store, dataset_or_plan_id).await?;
            let context = store.resolve_context(dataset_id).await?.with_context(|| {
                format!("dataset {dataset_id} has no approved authenticity binding")
            })?;
            presentation::print(&context)
        }
    }
}

async fn start(args: ResearchStartArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let brief = resolve_brief(store, &args.file).await?;
    let run = ResearchRun::queue(&brief, RESEARCH_PROTOCOL_VERSION, protocol_fingerprint()?)?;
    store.create_run(&brief, &run).await?;
    eprintln!(
        "research run {}: provider={}/{} external_calls={} limits: {} turns, {} searches, {} pages, {} bytes, {} seconds",
        run.id,
        brief.provider.provider,
        brief.provider.model,
        brief.provider.provider != "fake",
        brief.budgets.max_model_turns,
        brief.budgets.max_searches,
        brief.budgets.max_fetched_pages,
        brief.budgets.max_fetched_bytes,
        brief.budgets.max_wall_clock_seconds,
    );

    let sidecar = resolve_sidecar(&args.runtime)?;
    let mut runtime = PiProcessRuntime::new(args.runtime.node, sidecar);
    let (search, fetcher): (Arc<dyn SearchProvider>, Arc<dyn PageFetcher>) =
        if brief.provider.provider == "fake" {
            let script = args
                .script
                .context("--script is required for a fake research provider")?;
            let corpus = args
                .corpus
                .context("--corpus is required for a fake research provider")?;
            let scripted_turns: Value = read_json(&script)?;
            runtime = runtime.with_scripted_turns(scripted_turns);
            let corpus = Arc::new(CorpusWebAdapter::from_json(
                &std::fs::read(&corpus)
                    .with_context(|| format!("could not read corpus {}", corpus.display()))?,
            )?);
            (corpus.clone(), corpus)
        } else {
            ensure!(
                args.script.is_none() && args.corpus.is_none(),
                "--script and --corpus are only valid for a fake research provider"
            );
            let api_key = std::env::var(&args.search_api_key_env).with_context(|| {
                format!(
                    "search API key environment variable {} is not set",
                    args.search_api_key_env
                )
            })?;
            (
                Arc::new(BraveSearchProvider::new(api_key)?),
                Arc::new(HttpPageFetcher::new(brief.source_policy.clone())?),
            )
        };
    let runner = ResearchRunner::new(Arc::new(store.clone()), Arc::new(runtime), search, fetcher);
    let outcome = runner.run(run.id).await?;
    presentation::print(&outcome)
}

async fn recover(run_id: Uuid, store: &SqliteStore) -> anyhow::Result<()> {
    let mut run = require_run(store, run_id).await?;
    if run.state == ResearchRunState::Running {
        for mut call in store.list_tool_calls(run.id).await? {
            if call.state == ToolCallState::Started {
                call.interrupt()?;
                store.record_tool_call(&call).await?;
            }
        }
        run.fail(
            ResearchStopReason::Interrupted,
            "research host stopped; open tool calls were not replayed".into(),
        )?;
        store.save_run(&run).await?;
    }
    presentation::print(&run)
}

async fn review(args: ResearchReviewArgs, store: &SqliteStore) -> anyhow::Result<()> {
    let profile = store
        .get_profile(args.profile_id)
        .await?
        .with_context(|| format!("authenticity profile not found: {}", args.profile_id))?;
    let run = require_run(store, profile.run_id).await?;
    ensure!(
        run.state == ResearchRunState::AwaitingReview,
        "profile run is not awaiting review"
    );
    let predecessor = store.latest_review(profile.id).await?;
    let decision = if args.approve {
        ProfileReviewDecision::Approve
    } else if args.reject {
        ProfileReviewDecision::Reject
    } else {
        ProfileReviewDecision::RequestRevision
    };
    let review = ProfileReview::create(
        &profile,
        predecessor.as_ref(),
        decision,
        args.reviewer,
        args.reason,
    )?;
    store.append_review(&review).await?;
    presentation::print(&review)
}

async fn bind(store: &SqliteStore, target_id: Uuid, profile_id: Uuid) -> anyhow::Result<()> {
    let dataset_id = resolve_dataset_id(store, target_id).await?;
    let profile = store
        .get_profile(profile_id)
        .await?
        .with_context(|| format!("authenticity profile not found: {profile_id}"))?;
    ensure!(
        profile.dataset_id == dataset_id,
        "profile belongs to dataset {}, not {dataset_id}",
        profile.dataset_id
    );
    let approval = store
        .latest_review(profile_id)
        .await?
        .context("profile has no review")?;
    let predecessor_id = store
        .resolve_context(dataset_id)
        .await?
        .map(|context| context.binding_id);
    let binding = ProfileBinding::bind(&profile, &approval, predecessor_id)?;
    store.append_binding(&binding).await?;
    presentation::print(&binding)
}

async fn watch(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<()> {
    loop {
        let view = status(store, run_id).await?;
        eprintln!(
            "research {}: {:?}, turns={}, searches={}, pages={}, evidence={}",
            run_id,
            view.run.state,
            view.run.usage.model_turns,
            view.run.usage.searches,
            view.run.usage.fetched_pages,
            view.evidence_count,
        );
        if view.run.state != ResearchRunState::Queued && view.run.state != ResearchRunState::Running
        {
            return presentation::print(&view);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

#[derive(Debug, Serialize)]
struct ResearchStatus {
    run: ResearchRun,
    brief: ResolvedResearchBrief,
    tool_call_count: usize,
    tool_calls: Vec<ResearchToolCall>,
    evidence_count: usize,
    claim_count: usize,
    profile_id: Option<Uuid>,
}

async fn status(store: &SqliteStore, run_id: Uuid) -> anyhow::Result<ResearchStatus> {
    let run = require_run(store, run_id).await?;
    let brief = store
        .get_brief(run.brief_id)
        .await?
        .with_context(|| format!("research brief not found: {}", run.brief_id))?;
    let tool_calls = store.list_tool_calls(run_id).await?;
    Ok(ResearchStatus {
        tool_call_count: tool_calls.len(),
        tool_calls,
        evidence_count: store.list_evidence(run_id).await?.len(),
        claim_count: store.list_claims(run_id).await?.len(),
        profile_id: store
            .latest_profile_for_run(run_id)
            .await?
            .map(|profile| profile.id),
        run,
        brief,
    })
}

async fn require_run(store: &SqliteStore, id: Uuid) -> anyhow::Result<ResearchRun> {
    store
        .get_run(id)
        .await?
        .with_context(|| format!("research run not found: {id}"))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResearchBriefFile {
    schema_version: u32,
    #[serde(default)]
    dataset_id: Option<Uuid>,
    #[serde(default)]
    dataset_name: Option<String>,
    target: ResearchTarget,
    questions: Vec<String>,
    desired_source_diversity: u32,
    source_policy: SourcePolicy,
    budgets: ResearchBudgets,
    provider: ResearchProviderConfiguration,
    required_profile_sections: Vec<String>,
}

async fn resolve_brief(
    store: &SqliteStore,
    file: &PathBuf,
) -> anyhow::Result<ResolvedResearchBrief> {
    let input: ResearchBriefFile = read_document(file)?;
    ensure!(
        input.dataset_id.is_some() ^ input.dataset_name.is_some(),
        "research brief must specify exactly one of dataset_id or dataset_name"
    );
    let dataset = if let Some(id) = input.dataset_id {
        store
            .get_dataset(id)
            .await?
            .with_context(|| format!("dataset not found: {id}"))?
    } else {
        let name = input.dataset_name.as_deref().unwrap_or_default();
        let matches = store
            .list_datasets()
            .await?
            .into_iter()
            .filter(|dataset| dataset.name == name)
            .collect::<Vec<_>>();
        ensure!(
            matches.len() == 1,
            "dataset name {name:?} must resolve uniquely"
        );
        matches.into_iter().next().expect("one checked dataset")
    };
    let semantic = super::semantic::resolve_dataset_semantics(store, dataset.id).await?;
    ResolvedResearchBrief::create(ResearchBriefDraft {
        schema_version: input.schema_version,
        dataset: ArtifactReference {
            id: dataset.id,
            fingerprint: artifact_core::fingerprint(&dataset)?,
        },
        task: dataset.task_description,
        labels: dataset.labels,
        dimensions: dataset
            .dimensions
            .into_iter()
            .map(|dimension| (dimension.name, dimension.values))
            .collect::<BTreeMap<_, _>>(),
        semantic_context: Some(ArtifactReference {
            id: semantic.dataset_id,
            fingerprint: semantic.fingerprint,
        }),
        target: input.target,
        questions: input.questions,
        desired_source_diversity: input.desired_source_diversity,
        source_policy: input.source_policy,
        budgets: input.budgets,
        provider: input.provider,
        required_profile_sections: input.required_profile_sections,
    })
    .map_err(Into::into)
}

async fn resolve_dataset_id(store: &SqliteStore, id: Uuid) -> anyhow::Result<Uuid> {
    if store.get_dataset(id).await?.is_some() {
        return Ok(id);
    }
    store
        .get_plan(id)
        .await?
        .map(|plan| plan.dataset_id)
        .with_context(|| format!("neither dataset nor generation plan found: {id}"))
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
