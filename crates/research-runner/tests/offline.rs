use std::{
    collections::BTreeMap,
    collections::VecDeque,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use artifact_core::fingerprint;
use generation_core::{domain::DatasetDefinition, ports::DatasetStore};
use research_core::{
    brief::{
        ArtifactReference, ResearchBriefDraft, ResearchBudgets, ResearchProviderConfiguration,
        ResearchTarget, ResolvedResearchBrief, SourcePolicy,
    },
    lifecycle::{ResearchRun, ResearchRunState},
    ports::{
        AgentToolResult, BoxFuture, ResearchAdapterError, ResearchAgentEvent, ResearchAgentMessage,
        ResearchAgentRequest, ResearchAgentRuntime, ResearchAgentSession, ResearchStore,
        SearchProvider,
    },
};
use research_runner::{RESEARCH_PROTOCOL_VERSION, ResearchRunner, protocol_fingerprint};
use research_web::CorpusWebAdapter;
use serde_json::json;
use synthetic_data_sqlite::SqliteStore;
use tempfile::TempDir;
use uuid::Uuid;

#[tokio::test]
async fn iterative_offline_run_persists_evidence_claims_profile_and_final_turn_usage() {
    let temporary = tempfile::tempdir().expect("temporary directory");
    let store = Arc::new(
        SqliteStore::connect(&sqlite_url(&temporary, "research.db"))
            .await
            .expect("database"),
    );
    let dataset = DatasetDefinition::new(
        "support",
        "Classify support intent",
        vec!["billing".into()],
        vec![],
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("persist dataset");
    let brief = brief(&dataset);
    let run = ResearchRun::queue(
        &brief,
        RESEARCH_PROTOCOL_VERSION,
        protocol_fingerprint().expect("protocol fingerprint"),
    )
    .expect("queued run");
    store.create_run(&brief, &run).await.expect("persist run");

    let content = "Authentic requests are short, omit greetings, and sometimes contain typos.";
    let content_hash = fingerprint(&content).expect("content hash");
    let messages = VecDeque::from([
        event(ResearchAgentEvent::AgentText {
            text: "1. Search examples\n2. Compare evidence\n3. Draft profile".into(),
        }),
        tool(
            "search",
            "search_web",
            json!({"query":"authentic support","sourceClasses":["documentation"],"maximumResults":2}),
        ),
        event(turn(1)),
        tool(
            "fetch",
            "fetch_page",
            json!({"url":"https://example.com/support","maximumBytes":2000}),
        ),
        tool(
            "evidence",
            "record_evidence",
            json!({
                "key":"style",
                "url":"https://example.com/support",
                "title":"Authentic support",
                "query":"authentic support",
                "sourceClass":"documentation",
                "contentHash":content_hash,
                "excerpt":"short, omit greetings, and sometimes contain typos",
                "observation":"Requests are often short, greeting-free, and noisy.",
                "applicability":"Support intent inputs",
                "confidence":"high"
            }),
        ),
        tool(
            "draft",
            "draft_profile",
            json!({
                "claims":[{
                    "key":"style_claim",
                    "statement":"Authentic requests are often short and noisy.",
                    "confidence":"high",
                    "supportingEvidenceKeys":["style"],
                    "conflictingEvidenceKeys":[],
                    "inference":false
                }],
                "profile":{
                    "schemaVersion":1,
                    "summary":"Use short, occasionally noisy requests.",
                    "sections":{"language":{
                        "observations":["Messages can omit greetings."],
                        "generationInstructions":["Vary completeness and noise."],
                        "claimKeys":["style_claim"]
                    }},
                    "generationInstructions":["Do not make every row polished."],
                    "caveats":[]
                }
            }),
        ),
        tool(
            "finish",
            "finish_research",
            json!({"reason":"sufficient_evidence","summary":"ready"}),
        ),
        event(turn(2)),
        ResearchAgentMessage::Completed,
    ]);
    let runtime = Arc::new(ScriptedRuntime::new(messages));
    let corpus = Arc::new(
        CorpusWebAdapter::from_json(
            serde_json::to_string(&json!([{
                "url":"https://example.com/support",
                "title":"Authentic support",
                "source_class":"documentation",
                "tags":["authentic support"],
                "content":content
            }]))
            .expect("corpus JSON")
            .as_bytes(),
        )
        .expect("corpus"),
    );
    let search = Arc::new(FailOnceSearch {
        inner: corpus.clone(),
        attempts: AtomicUsize::new(0),
    });
    let runner = ResearchRunner::new(store.clone(), runtime, search.clone(), corpus);
    let outcome = runner.run(run.id).await.expect("research completes");

    assert_eq!(outcome.run.state, ResearchRunState::AwaitingReview);
    assert_eq!(outcome.run.usage.model_turns, 2);
    assert_eq!(outcome.run.usage.searches, 2);
    assert_eq!(search.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(outcome.run.usage.fetched_pages, 1);
    assert_eq!(
        store.list_evidence(run.id).await.expect("evidence").len(),
        1
    );
    assert_eq!(store.list_claims(run.id).await.expect("claims").len(), 1);
    let profile = outcome.profile.expect("profile");
    assert_eq!(profile.claims.len(), 1);
    assert!(profile.sections.contains_key("language"));
}

struct FailOnceSearch {
    inner: Arc<CorpusWebAdapter>,
    attempts: AtomicUsize,
}

impl SearchProvider for FailOnceSearch {
    fn search(
        &self,
        request: research_core::evidence::SearchRequest,
    ) -> BoxFuture<'_, Result<Vec<research_core::evidence::SearchResult>, ResearchAdapterError>>
    {
        Box::pin(async move {
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(ResearchAdapterError("transient search failure".into()));
            }
            self.inner.search(request).await
        })
    }
}

fn brief(dataset: &DatasetDefinition) -> ResolvedResearchBrief {
    ResolvedResearchBrief::create(ResearchBriefDraft {
        schema_version: 1,
        dataset: ArtifactReference {
            id: dataset.id,
            fingerprint: fingerprint(dataset).expect("dataset fingerprint"),
        },
        task: dataset.task_description.clone(),
        labels: dataset.labels.clone(),
        dimensions: BTreeMap::new(),
        semantic_context: None,
        target: ResearchTarget {
            language: "English".into(),
            ..ResearchTarget::default()
        },
        questions: vec!["How are real support requests written?".into()],
        desired_source_diversity: 1,
        source_policy: SourcePolicy {
            allowed_domains: vec!["example.com".into()],
            blocked_domains: vec![],
            allowed_source_classes: vec!["documentation".into()],
        },
        budgets: ResearchBudgets {
            max_model_turns: 4,
            max_searches: 2,
            max_fetched_pages: 2,
            max_fetched_bytes: 10_000,
            max_input_tokens: 10_000,
            max_output_tokens: 10_000,
            max_wall_clock_seconds: 10,
            max_cost_microusd: 0,
            max_retries_per_call: 1,
        },
        provider: ResearchProviderConfiguration {
            runtime: "pi".into(),
            provider: "fake".into(),
            model: "scripted".into(),
            api_key_env: None,
        },
        required_profile_sections: vec!["language".into()],
    })
    .expect("brief")
}

fn event(event: ResearchAgentEvent) -> ResearchAgentMessage {
    ResearchAgentMessage::Event { event }
}

fn turn(sequence: u32) -> ResearchAgentEvent {
    ResearchAgentEvent::ModelTurnCompleted {
        sequence,
        input_tokens: 10,
        output_tokens: 5,
        cost_microusd: 0,
    }
}

fn tool(call: &str, name: &str, arguments: serde_json::Value) -> ResearchAgentMessage {
    ResearchAgentMessage::ToolRequest {
        request: research_core::ports::AgentToolRequest {
            external_call_id: call.into(),
            name: name.into(),
            arguments,
        },
    }
}

struct ScriptedRuntime {
    messages: std::sync::Mutex<Option<VecDeque<ResearchAgentMessage>>>,
}

impl ScriptedRuntime {
    fn new(messages: VecDeque<ResearchAgentMessage>) -> Self {
        Self {
            messages: std::sync::Mutex::new(Some(messages)),
        }
    }
}

impl ResearchAgentRuntime for ScriptedRuntime {
    fn start(
        &self,
        _request: ResearchAgentRequest,
    ) -> BoxFuture<'_, Result<Box<dyn ResearchAgentSession>, ResearchAdapterError>> {
        Box::pin(async move {
            let messages = self
                .messages
                .lock()
                .expect("script lock")
                .take()
                .ok_or_else(|| ResearchAdapterError("script already used".into()))?;
            Ok(Box::new(ScriptedSession { messages }) as Box<dyn ResearchAgentSession>)
        })
    }
}

struct ScriptedSession {
    messages: VecDeque<ResearchAgentMessage>,
}

impl ResearchAgentSession for ScriptedSession {
    fn next_message(
        &mut self,
    ) -> BoxFuture<'_, Result<ResearchAgentMessage, ResearchAdapterError>> {
        Box::pin(async move {
            self.messages
                .pop_front()
                .ok_or_else(|| ResearchAdapterError("script ended".into()))
        })
    }

    fn send_tool_result(
        &mut self,
        _external_call_id: &str,
        _result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        Box::pin(async { Ok(()) })
    }

    fn send_tool_error(
        &mut self,
        _external_call_id: &str,
        message: &str,
    ) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        let message = message.to_owned();
        Box::pin(async move { Err(ResearchAdapterError(message)) })
    }

    fn cancel(&mut self, _run_id: Uuid) -> BoxFuture<'_, Result<(), ResearchAdapterError>> {
        Box::pin(async { Ok(()) })
    }
}

fn sqlite_url(directory: &TempDir, name: &str) -> String {
    let path = directory.path().join(name);
    format!("sqlite://{}?mode=rwc", slash_path(&path))
}

fn slash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
