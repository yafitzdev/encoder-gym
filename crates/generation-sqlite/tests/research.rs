use std::collections::BTreeMap;

use artifact_core::fingerprint;
use generation_core::{domain::DatasetDefinition, ports::DatasetStore};
use research_core::{
    brief::{
        ArtifactReference, ResearchBriefDraft, ResearchBudgets, ResearchProviderConfiguration,
        ResearchTarget, ResolvedResearchBrief, SourcePolicy,
    },
    evidence::{
        EvidenceConfidence, ResearchClaim, ResearchClaimDraft, ResearchEvidence,
        ResearchEvidenceDraft,
    },
    lifecycle::{
        ResearchRun, ResearchStopReason, ResearchToolCall, ResearchToolKind, ResearchUsage,
    },
    ports::ResearchStore,
    profile::{
        AuthenticityProfile, AuthenticityProfileDraft, AuthenticitySection, ProfileBinding,
        ProfileReview, ProfileReviewDecision,
    },
};
use synthetic_data_sqlite::SqliteStore;

async fn store() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("research.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
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
            channel: Some("support chat".into()),
            ..ResearchTarget::default()
        },
        questions: vec!["How do authentic messages express urgency?".into()],
        desired_source_diversity: 2,
        source_policy: SourcePolicy::default(),
        budgets: ResearchBudgets {
            max_model_turns: 8,
            max_searches: 8,
            max_fetched_pages: 16,
            max_fetched_bytes: 1_000_000,
            max_input_tokens: 50_000,
            max_output_tokens: 10_000,
            max_wall_clock_seconds: 600,
            max_cost_microusd: 100_000,
            max_retries_per_call: 2,
        },
        provider: ResearchProviderConfiguration {
            runtime: "pi".into(),
            provider: "fake".into(),
            model: "scripted".into(),
            api_key_env: None,
        },
        required_profile_sections: vec!["language".into()],
    })
    .expect("resolved brief")
}

#[tokio::test]
async fn research_artifacts_reviews_and_binding_are_durable_and_append_only() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new(
        "support",
        "Classify support requests",
        vec!["billing".into(), "fraud".into()],
        vec![],
    )
    .expect("dataset");
    store
        .create_dataset(&dataset)
        .await
        .expect("persist dataset");
    let brief = brief(&dataset);
    let mut run = ResearchRun::queue(&brief, 1, "sha256:pi-protocol".into()).expect("run");
    store.create_run(&brief, &run).await.expect("create run");
    assert_eq!(
        store.get_brief(brief.id).await.unwrap(),
        Some(brief.clone())
    );
    assert_eq!(store.get_run(run.id).await.unwrap(), Some(run.clone()));

    run.start().expect("start");
    run.set_plan(vec![
        "Search authentic examples".into(),
        "Synthesize profile".into(),
    ])
    .expect("plan");
    store.save_run(&run).await.expect("save running run");

    let mut call = ResearchToolCall::start(
        &run,
        1,
        ResearchToolKind::FetchPage,
        serde_json::json!({"url": "https://example.com/support"}),
    )
    .expect("tool call");
    store.record_tool_call(&call).await.expect("started call");
    call.succeed(
        serde_json::json!({"content_hash": "sha256:page"}),
        ResearchUsage {
            fetched_pages: 1,
            fetched_bytes: 120,
            ..ResearchUsage::default()
        },
    )
    .expect("finish call");
    store.record_tool_call(&call).await.expect("finished call");
    assert_eq!(
        store.list_tool_calls(run.id).await.unwrap(),
        vec![call.clone()]
    );

    let evidence = ResearchEvidence::create(
        run.id,
        call.id,
        &brief.source_policy,
        ResearchEvidenceDraft {
            url: "https://example.com/support#messages".into(),
            title: "Support examples".into(),
            query: "authentic support language".into(),
            source_class: "documentation".into(),
            content_hash: "sha256:page".into(),
            excerpt: "charged twice pls help".into(),
            location: Some("messages".into()),
            observation: "Messages use fragments and omit greetings.".into(),
            applicability: "English support chat".into(),
            confidence: EvidenceConfidence::High,
        },
    )
    .expect("evidence");
    store
        .record_evidence(&evidence)
        .await
        .expect("persist evidence");
    let claim = ResearchClaim::create(
        run.id,
        std::slice::from_ref(&evidence),
        ResearchClaimDraft {
            statement: "Authentic support messages often use short fragments.".into(),
            confidence: EvidenceConfidence::High,
            supporting_evidence_ids: vec![evidence.id],
            conflicting_evidence_ids: vec![],
            inference: false,
        },
    )
    .expect("claim");
    store.record_claim(&claim).await.expect("persist claim");
    assert_eq!(
        store.list_evidence(run.id).await.unwrap(),
        vec![evidence.clone()]
    );
    assert_eq!(
        store.list_claims(run.id).await.unwrap(),
        vec![claim.clone()]
    );

    run.await_review(ResearchStopReason::SufficientEvidence)
        .expect("await review");
    store.save_run(&run).await.expect("save completed research");
    let profile = AuthenticityProfile::create(
        &brief,
        &run,
        None,
        std::slice::from_ref(&evidence),
        vec![claim.clone()],
        AuthenticityProfileDraft {
            schema_version: 1,
            predecessor_id: None,
            summary: "Support chat is terse and context-dependent.".into(),
            sections: BTreeMap::from([(
                "language".into(),
                AuthenticitySection {
                    observations: vec!["Short fragments are common.".into()],
                    generation_instructions: vec!["Use occasional grammatical fragments.".into()],
                    claim_ids: vec![claim.id],
                },
            )]),
            generation_instructions: vec!["Do not copy source wording.".into()],
            caveats: vec!["Evidence corpus is small.".into()],
        },
    )
    .expect("profile");
    store.save_profile(&profile).await.expect("persist profile");
    assert_eq!(
        store.get_profile(profile.id).await.unwrap(),
        Some(profile.clone())
    );

    let rejection = ProfileReview::create(
        &profile,
        None,
        ProfileReviewDecision::RequestRevision,
        "operator".into(),
        "Clarify the channel caveat.".into(),
    )
    .expect("revision review");
    store
        .append_review(&rejection)
        .await
        .expect("append review");
    let approval = ProfileReview::create(
        &profile,
        Some(&rejection),
        ProfileReviewDecision::Approve,
        "operator".into(),
        "Approved for the pilot.".into(),
    )
    .expect("approval");
    store
        .append_review(&approval)
        .await
        .expect("append approval");
    assert_eq!(
        store.latest_review(profile.id).await.unwrap(),
        Some(approval.clone())
    );

    let stale = ProfileReview::create(
        &profile,
        Some(&rejection),
        ProfileReviewDecision::Reject,
        "operator".into(),
        "stale decision".into(),
    )
    .expect("stale review shape");
    assert!(store.append_review(&stale).await.is_err());

    let binding = ProfileBinding::bind(&profile, &approval, None).expect("binding");
    store
        .append_binding(&binding)
        .await
        .expect("append binding");
    let context = store
        .resolve_context(dataset.id)
        .await
        .expect("resolve")
        .expect("bound context");
    assert_eq!(context.profile_fingerprint, profile.fingerprint);
    assert_eq!(context.binding_fingerprint, binding.fingerprint);
    assert_eq!(
        context.reproduce_fingerprint().unwrap(),
        context.fingerprint
    );
}

#[tokio::test]
async fn duplicate_source_content_is_rejected_within_a_run() {
    let (_directory, store) = store().await;
    let dataset = DatasetDefinition::new("support", "task", vec!["a".into()], vec![]).unwrap();
    store.create_dataset(&dataset).await.unwrap();
    let brief = brief(&dataset);
    let mut run = ResearchRun::queue(&brief, 1, "sha256:protocol".into()).unwrap();
    store.create_run(&brief, &run).await.unwrap();
    run.start().unwrap();
    store.save_run(&run).await.unwrap();
    let call = ResearchToolCall::start(
        &run,
        1,
        ResearchToolKind::RecordEvidence,
        serde_json::json!({}),
    )
    .unwrap();
    store.record_tool_call(&call).await.unwrap();
    let make = || {
        ResearchEvidence::create(
            run.id,
            call.id,
            &brief.source_policy,
            ResearchEvidenceDraft {
                url: "https://example.com/source".into(),
                title: "source".into(),
                query: "query".into(),
                source_class: "documentation".into(),
                content_hash: "sha256:same".into(),
                excerpt: "excerpt".into(),
                location: None,
                observation: "observation".into(),
                applicability: "applicability".into(),
                confidence: EvidenceConfidence::Medium,
            },
        )
        .unwrap()
    };
    store.record_evidence(&make()).await.unwrap();
    assert!(store.record_evidence(&make()).await.is_err());
}
