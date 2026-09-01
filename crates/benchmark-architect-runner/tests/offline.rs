use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};

use benchmark_architect_core::{
    blueprint::{
        BenchmarkAcquisitionHandoff, BenchmarkArchitectureProposal, BenchmarkArchitectureReview,
        BenchmarkBlueprintDraft, BenchmarkCohortBlueprint, BenchmarkSuiteBlueprint,
        CohortFreshnessPolicy, RiskCoverageRequirement,
    },
    brief::{
        BenchmarkArchitectBriefDraft, BenchmarkArchitectBudgets, BenchmarkArchitectProvider,
        BenchmarkObjective, DeploymentContext, DeploymentRisk, LabelSemantic,
        ResolvedBenchmarkArchitectBrief, RiskLevel,
    },
    lifecycle::{BenchmarkArchitectRun, BenchmarkArchitectRunState, BenchmarkArchitectToolCall},
    ports::{
        AgentToolRequest, AgentToolResult, BenchmarkArchitectAdapterError,
        BenchmarkArchitectAgentEvent, BenchmarkArchitectAgentMessage,
        BenchmarkArchitectAgentRequest, BenchmarkArchitectAgentRuntime,
        BenchmarkArchitectAgentSession, BenchmarkArchitectStore, BoxFuture, PageFetcher,
        SearchProvider,
    },
};
use benchmark_architect_runner::BenchmarkArchitectRunner;
use evaluation_core::domain::EvaluationProtocol;
use research_core::{
    brief::SourcePolicy,
    evidence::{FetchRequest, ResearchEvidence, SearchRequest, SearchResult, UntrustedPage},
};
use serde_json::json;
use uuid::Uuid;
use workflow_core::{
    benchmark::{
        AcceptanceContract, BenchmarkMetric, BenchmarkSuiteKind, MetricRequirement, MetricTarget,
        RegressionRequirement,
    },
    benchmark_qualification::{BenchmarkQualificationPolicy, QualificationConfidence},
    governance::{CohortOrigin, CohortRole, DisclosureLevel},
};

#[tokio::test]
async fn bounded_agent_researches_previews_and_submits_a_row_free_blueprint() {
    let brief = brief();
    let source = Arc::new(SourceFixture::new());
    let raw_source = source.content.clone();
    let blueprint = blueprint();
    let evidence_binding = json!([{
        "requirementKey": "fraud-boundary",
        "supportingEvidenceKeys": ["risk-study"],
        "conflictingEvidenceKeys": [],
    }]);
    let messages = VecDeque::from([
        event(BenchmarkArchitectAgentEvent::AgentText {
            text: "Inspect the row-free brief, research risks, preview, and submit.".into(),
        }),
        tool("inspect_brief", json!({})),
        turn(1),
        tool(
            "search_web",
            json!({
                "query": "support fraud billing confusion",
                "sourceClasses": ["industry_report"],
                "maximumResults": 3,
            }),
        ),
        turn(2),
        tool(
            "fetch_page",
            json!({"url": source.url, "maximumBytes": 10000}),
        ),
        turn(3),
        tool(
            "record_evidence",
            json!({
                "key": "risk-study",
                "url": source.url,
                "title": source.title,
                "query": "support fraud billing confusion",
                "sourceClass": "industry_report",
                "contentHash": source.content_hash,
                "excerpt": source.excerpt,
                "observation": "Fraud and billing descriptions overlap in real support intake.",
                "applicability": "Require dedicated fraud/billing boundary cohorts.",
                "confidence": "high",
            }),
        ),
        turn(4),
        tool(
            "preview_blueprint",
            json!({"blueprint": blueprint, "evidenceBindings": evidence_binding}),
        ),
        turn(5),
        tool(
            "submit_blueprint",
            json!({"blueprint": blueprint, "evidenceBindings": evidence_binding}),
        ),
        turn(6),
        tool(
            "finish_benchmark_architecture",
            json!({
                "reason": "proposal_submitted",
                "summary": "Evidence-backed blueprint submitted."
            }),
        ),
        turn(7),
        BenchmarkArchitectAgentMessage::Completed,
    ]);
    let store = Arc::new(MemoryStore::default());
    let runner = BenchmarkArchitectRunner::new(
        Arc::new(ScriptedRuntime(Mutex::new(Some(messages)))),
        source.clone(),
        source,
        store.clone(),
    );
    let queued = runner.queue(brief).await.unwrap();
    let outcome = runner.run(queued.id).await.unwrap();

    assert_eq!(
        outcome.run.state,
        BenchmarkArchitectRunState::AwaitingReview
    );
    assert_eq!(outcome.run.usage.model_turns, 7);
    assert_eq!(outcome.run.usage.tool_calls, 7);
    assert_eq!(outcome.run.usage.searches, 1);
    assert_eq!(outcome.run.usage.fetched_pages, 1);
    assert_eq!(outcome.run.usage.blueprint_previews, 1);
    assert_eq!(store.calls.lock().unwrap().len(), 7);
    assert_eq!(store.evidence.lock().unwrap().len(), 1);
    let proposal = outcome.proposal.unwrap();
    assert_eq!(proposal.evidence_fingerprints.len(), 1);
    assert_eq!(proposal.acquisition_requirements.len(), 2);
    assert_eq!(proposal.risk_coverage[0].supporting_evidence_ids.len(), 1);
    let serialized = serde_json::to_string(&proposal).unwrap();
    assert!(!serialized.contains(&raw_source));
}

fn brief() -> ResolvedBenchmarkArchitectBrief {
    ResolvedBenchmarkArchitectBrief::create(BenchmarkArchitectBriefDraft {
        task: "Classify support messages".into(),
        labels: vec!["billing".into(), "fraud".into()],
        label_semantics: vec![
            LabelSemantic {
                label: "billing".into(),
                meaning: "Invoices and legitimate charges".into(),
                inclusions: vec![],
                exclusions: vec![],
            },
            LabelSemantic {
                label: "fraud".into(),
                meaning: "Unauthorized account activity".into(),
                inclusions: vec![],
                exclusions: vec![],
            },
        ],
        deployment: DeploymentContext {
            summary: "Consumer support intake".into(),
            users: vec!["customers".into()],
            channels: vec!["chat".into()],
            languages: vec!["English".into()],
            regions: vec![],
            time_horizon: Some("next year".into()),
            constraints: vec![],
        },
        risks: vec![DeploymentRisk {
            key: "fraud_as_billing".into(),
            description: "Fraud is routed as a routine billing issue".into(),
            consequence: "Account protection is delayed".into(),
            likelihood: RiskLevel::Medium,
            severity: RiskLevel::Critical,
            weight: 100,
        }],
        objectives: vec![BenchmarkObjective {
            key: "boundary_quality".into(),
            description: "Measure the fraud/billing boundary".into(),
            weight: 100,
        }],
        candidate_sources: vec![],
        existing_benchmark: None,
        source_policy: SourcePolicy {
            allowed_domains: vec!["example.com".into()],
            blocked_domains: vec![],
            allowed_source_classes: vec!["industry_report".into()],
        },
        budgets: BenchmarkArchitectBudgets {
            max_model_turns: 10,
            max_tool_calls: 20,
            max_searches: 3,
            max_fetched_pages: 3,
            max_fetched_bytes: 50_000,
            max_blueprint_previews: 3,
            max_input_tokens: 100_000,
            max_output_tokens: 20_000,
            max_cost_microusd: 1_000_000,
            max_wall_clock_seconds: 30,
        },
        provider: BenchmarkArchitectProvider {
            runtime: "pi".into(),
            provider: "fake".into(),
            model: "scripted".into(),
            api_key_env: None,
        },
    })
    .unwrap()
}

fn blueprint() -> BenchmarkBlueprintDraft {
    BenchmarkBlueprintDraft {
        summary: "Use adaptive development and single-use sealed boundary evidence.".into(),
        suites: vec![
            suite(
                BenchmarkSuiteKind::Development,
                "development",
                "dev-boundary",
                CohortRole::Development,
            ),
            suite(
                BenchmarkSuiteKind::SealedAcceptance,
                "sealed",
                "sealed-boundary",
                CohortRole::SealedAcceptance,
            ),
        ],
        qualification_policy: BenchmarkQualificationPolicy {
            minimum_overall_support: 100,
            minimum_label_support: 20,
            confidence: QualificationConfidence::NinetyFive,
            maximum_proportion_margin_of_error: 0.10,
            maximum_normalized_duplicate_rate: 0.01,
            minimum_distinct_producers: 2,
            maximum_single_producer_share: 0.8,
            maximum_label_imbalance_ratio: 2.0,
        },
        risk_coverage: vec![RiskCoverageRequirement {
            key: "fraud-boundary".into(),
            deployment_risk_key: "fraud_as_billing".into(),
            cohort_keys: vec!["dev-boundary".into(), "sealed-boundary".into()],
            rationale: "Both adaptive and final evidence cover this failure.".into(),
            supporting_evidence_ids: vec![],
            conflicting_evidence_ids: vec![],
            inference: false,
        }],
        tradeoffs: vec!["Boundary density reduces easy-example share.".into()],
        evidence_gaps: vec!["True production prevalence is unknown.".into()],
    }
}

fn suite(
    kind: BenchmarkSuiteKind,
    name: &str,
    key: &str,
    role: CohortRole,
) -> BenchmarkSuiteBlueprint {
    BenchmarkSuiteBlueprint {
        kind,
        name: name.into(),
        purpose: if kind == BenchmarkSuiteKind::Development {
            "Guide bounded development".into()
        } else {
            "Make one final acceptance decision".into()
        },
        required_model_formats: vec![],
        cohorts: vec![BenchmarkCohortBlueprint {
            key: key.into(),
            name: key.into(),
            purpose: "Measure production-like class boundaries".into(),
            origin: CohortOrigin::InternalSnapshot,
            role,
            disclosure: DisclosureLevel::Aggregate,
            adaptation_eligible: kind == BenchmarkSuiteKind::Development,
            protocol: EvaluationProtocol {
                minimum_slice_support: 20,
                ..EvaluationProtocol::default()
            },
            minimum_total_support: 400,
            minimum_label_support: BTreeMap::from([("billing".into(), 200), ("fraud".into(), 200)]),
            required_dimension_values: BTreeMap::from([("channel".into(), vec!["chat".into()])]),
            required_slice_support: BTreeMap::new(),
            required_source_classes: vec!["reviewed_real_sample".into()],
            minimum_distinct_producers: 2,
            freshness: CohortFreshnessPolicy {
                maximum_age_days: 180,
                maximum_workflow_iterations: 3,
                maximum_adaptive_exposures: if kind == BenchmarkSuiteKind::Development {
                    10
                } else {
                    0
                },
                maximum_acceptance_exposures: if kind == BenchmarkSuiteKind::SealedAcceptance {
                    1
                } else {
                    0
                },
            },
            rationale: "High-consequence boundary deserves direct evidence.".into(),
        }],
        contract: AcceptanceContract {
            metric_requirements: vec![MetricRequirement {
                target: MetricTarget::Overall,
                metric: BenchmarkMetric::MacroF1,
                minimum: Some(0.85),
                maximum: None,
                minimum_support: 100,
            }],
            regression: (kind == BenchmarkSuiteKind::Development).then_some(
                RegressionRequirement {
                    max_accuracy_drop: Some(0.02),
                    max_macro_f1_drop: None,
                    minimum_accuracy_delta_lower_bound: None,
                    minimum_macro_f1_delta_lower_bound: None,
                    require_mcnemar_significance: false,
                },
            ),
        },
    }
}

fn event(event: BenchmarkArchitectAgentEvent) -> BenchmarkArchitectAgentMessage {
    BenchmarkArchitectAgentMessage::Event { event }
}

fn turn(sequence: u32) -> BenchmarkArchitectAgentMessage {
    event(BenchmarkArchitectAgentEvent::ModelTurnCompleted {
        sequence,
        input_tokens: 100,
        output_tokens: 50,
        cost_microusd: 100,
    })
}

fn tool(name: &str, arguments: serde_json::Value) -> BenchmarkArchitectAgentMessage {
    BenchmarkArchitectAgentMessage::ToolRequest {
        request: AgentToolRequest {
            external_call_id: format!("call-{name}"),
            name: name.into(),
            arguments,
        },
    }
}

struct ScriptedRuntime(Mutex<Option<VecDeque<BenchmarkArchitectAgentMessage>>>);

impl BenchmarkArchitectAgentRuntime for ScriptedRuntime {
    fn start(
        &self,
        request: BenchmarkArchitectAgentRequest,
    ) -> BoxFuture<
        '_,
        Result<Box<dyn BenchmarkArchitectAgentSession>, BenchmarkArchitectAdapterError>,
    > {
        Box::pin(async move {
            assert_eq!(request.capability_set, "benchmark_architect_v1");
            assert!(!request.initial_prompt.contains("row_text"));
            let messages =
                self.0.lock().unwrap().take().ok_or_else(|| {
                    BenchmarkArchitectAdapterError("script already consumed".into())
                })?;
            Ok(Box::new(ScriptedSession(messages)) as Box<dyn BenchmarkArchitectAgentSession>)
        })
    }
}

struct ScriptedSession(VecDeque<BenchmarkArchitectAgentMessage>);

impl BenchmarkArchitectAgentSession for ScriptedSession {
    fn next_message(
        &mut self,
    ) -> BoxFuture<'_, Result<BenchmarkArchitectAgentMessage, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            self.0
                .pop_front()
                .ok_or_else(|| BenchmarkArchitectAdapterError("script ended".into()))
        })
    }

    fn send_tool_result(
        &mut self,
        _external_call_id: &str,
        _result: AgentToolResult,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        Box::pin(async { Ok(()) })
    }

    fn send_tool_error(
        &mut self,
        _external_call_id: &str,
        message: &str,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let message = message.to_owned();
        Box::pin(async move { Err(BenchmarkArchitectAdapterError(message)) })
    }

    fn cancel(
        &mut self,
        _run_id: Uuid,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        Box::pin(async { Ok(()) })
    }
}

struct SourceFixture {
    url: String,
    title: String,
    content: String,
    content_hash: String,
    excerpt: String,
}

impl SourceFixture {
    fn new() -> Self {
        let content = "Observed failure: users often describe unauthorized charges using ordinary billing language. Additional discussion.".to_owned();
        Self {
            url: "https://example.com/fraud-support-study".into(),
            title: "Fraud support study".into(),
            content_hash: artifact_core::fingerprint(&content).unwrap(),
            excerpt: "users often describe unauthorized charges using ordinary billing language"
                .into(),
            content,
        }
    }
}

impl SearchProvider for SourceFixture {
    fn search(
        &self,
        _request: SearchRequest,
    ) -> BoxFuture<'_, Result<Vec<SearchResult>, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            Ok(vec![SearchResult {
                url: self.url.clone(),
                title: self.title.clone(),
                summary: "Real support language overlaps at the fraud/billing boundary.".into(),
                source_class: Some("industry_report".into()),
            }])
        })
    }
}

impl PageFetcher for SourceFixture {
    fn fetch(
        &self,
        _request: FetchRequest,
    ) -> BoxFuture<'_, Result<UntrustedPage, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            UntrustedPage::create(
                &SourcePolicy::default(),
                self.url.clone(),
                self.title.clone(),
                Some("text/plain".into()),
                self.content.clone(),
            )
            .map_err(|error| BenchmarkArchitectAdapterError(error.to_string()))
        })
    }
}

#[derive(Default)]
struct MemoryStore {
    brief: Mutex<Option<ResolvedBenchmarkArchitectBrief>>,
    run: Mutex<Option<BenchmarkArchitectRun>>,
    calls: Mutex<Vec<BenchmarkArchitectToolCall>>,
    evidence: Mutex<Vec<ResearchEvidence>>,
    proposal: Mutex<Option<BenchmarkArchitectureProposal>>,
    reviews: Mutex<Vec<BenchmarkArchitectureReview>>,
    handoff: Mutex<Option<BenchmarkAcquisitionHandoff>>,
}

impl BenchmarkArchitectStore for MemoryStore {
    fn create_run(
        &self,
        brief: &ResolvedBenchmarkArchitectBrief,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let brief = brief.clone();
        let run = run.clone();
        Box::pin(async move {
            *self.brief.lock().unwrap() = Some(brief);
            *self.run.lock().unwrap() = Some(run);
            Ok(())
        })
    }

    fn get_brief(
        &self,
        id: Uuid,
    ) -> BoxFuture<
        '_,
        Result<Option<ResolvedBenchmarkArchitectBrief>, BenchmarkArchitectAdapterError>,
    > {
        Box::pin(async move {
            Ok(self
                .brief
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.id == id))
        })
    }

    fn get_run(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectRun>, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .run
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.id == id))
        })
    }

    fn save_run(
        &self,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let run = run.clone();
        Box::pin(async move {
            *self.run.lock().unwrap() = Some(run);
            Ok(())
        })
    }

    fn record_tool_call(
        &self,
        call: &BenchmarkArchitectToolCall,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let call = call.clone();
        Box::pin(async move {
            let mut calls = self.calls.lock().unwrap();
            if let Some(existing) = calls.iter_mut().find(|value| value.id == call.id) {
                *existing = call;
            } else {
                calls.push(call);
            }
            Ok(())
        })
    }

    fn list_tool_calls(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<BenchmarkArchitectToolCall>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            Ok(self
                .calls
                .lock()
                .unwrap()
                .iter()
                .filter(|value| value.run_id == run_id)
                .cloned()
                .collect())
        })
    }

    fn record_evidence(
        &self,
        evidence: &ResearchEvidence,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let evidence = evidence.clone();
        Box::pin(async move {
            self.evidence.lock().unwrap().push(evidence);
            Ok(())
        })
    }

    fn list_evidence(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Vec<ResearchEvidence>, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .evidence
                .lock()
                .unwrap()
                .iter()
                .filter(|value| value.run_id == run_id)
                .cloned()
                .collect())
        })
    }

    fn get_evidence(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<ResearchEvidence>, BenchmarkArchitectAdapterError>> {
        Box::pin(async move {
            Ok(self
                .evidence
                .lock()
                .unwrap()
                .iter()
                .find(|value| value.id == id)
                .cloned())
        })
    }

    fn save_proposal_and_run(
        &self,
        proposal: &BenchmarkArchitectureProposal,
        run: &BenchmarkArchitectRun,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let proposal = proposal.clone();
        let run = run.clone();
        Box::pin(async move {
            *self.proposal.lock().unwrap() = Some(proposal);
            *self.run.lock().unwrap() = Some(run);
            Ok(())
        })
    }

    fn get_proposal(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureProposal>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            Ok(self
                .proposal
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.id == id))
        })
    }

    fn latest_proposal_for_run(
        &self,
        run_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureProposal>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            Ok(self
                .proposal
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.run_id == run_id))
        })
    }

    fn append_review(
        &self,
        review: &BenchmarkArchitectureReview,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let review = review.clone();
        Box::pin(async move {
            self.reviews.lock().unwrap().push(review);
            Ok(())
        })
    }

    fn latest_review(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureReview>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            Ok(self
                .reviews
                .lock()
                .unwrap()
                .iter()
                .filter(|value| value.proposal_id == proposal_id)
                .max_by_key(|value| (value.created_at, value.id))
                .cloned())
        })
    }

    fn get_review(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkArchitectureReview>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            Ok(self
                .reviews
                .lock()
                .unwrap()
                .iter()
                .find(|value| value.id == id)
                .cloned())
        })
    }

    fn save_handoff(
        &self,
        handoff: &BenchmarkAcquisitionHandoff,
    ) -> BoxFuture<'_, Result<(), BenchmarkArchitectAdapterError>> {
        let handoff = handoff.clone();
        Box::pin(async move {
            *self.handoff.lock().unwrap() = Some(handoff);
            Ok(())
        })
    }

    fn get_handoff(
        &self,
        proposal_id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkAcquisitionHandoff>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            Ok(self
                .handoff
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.proposal_id == proposal_id))
        })
    }

    fn get_handoff_by_id(
        &self,
        id: Uuid,
    ) -> BoxFuture<'_, Result<Option<BenchmarkAcquisitionHandoff>, BenchmarkArchitectAdapterError>>
    {
        Box::pin(async move {
            Ok(self
                .handoff
                .lock()
                .unwrap()
                .clone()
                .filter(|value| value.id == id))
        })
    }
}
