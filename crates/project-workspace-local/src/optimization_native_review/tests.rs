use std::collections::BTreeMap;

use dataset_quality_core::native_assessment::{
    NativeBlindAssessmentDraft, NativeBlindAssessmentEvidence, NativeBlindAssessmentRequest,
    NativeBlindContext, NativeBlindRow, NativeCandidateSemantics, NativeReviewCallOutcome,
    NativeReviewCallReservation, NativeReviewOperationCategory, NativeReviewRequest,
    NativeReviewResponse, NativeReviewUsage,
};
use project_workspace_core::ProviderLimits;
use serde_json::Value;
use sqlx::SqliteConnection;

use super::*;

async fn fixture() -> (SqliteConnection, NativeBlindAssessmentRequest) {
    let mut database = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql("PRAGMA foreign_keys=ON; CREATE TABLE project_optimization_runs(id TEXT PRIMARY KEY); CREATE TABLE project_optimization_events(run_id TEXT, sequence INTEGER, kind TEXT);")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0013_optimization_agent.sql"))
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/0025_optimization_native_review.sql"
    ))
    .execute(&mut database)
    .await
    .unwrap();
    let run_id = Uuid::new_v4();
    sqlx::query("INSERT INTO project_optimization_runs VALUES (?)")
        .bind(run_id.to_string())
        .execute(&mut database)
        .await
        .unwrap();
    let row = NativeBlindRow {
        row_id: "generated:0".into(),
        row_fingerprint: fingerprint(&"row").unwrap(),
        question: "Find the saved invoice".into(),
        context: NativeBlindContext::new(BTreeMap::from([(
            "task_kind".into(),
            Value::String("retrieval".into()),
        )]))
        .unwrap(),
        candidates: vec![
            NativeCandidateSemantics::new("read", "Read a saved record", vec!["retrieve".into()])
                .unwrap(),
            NativeCandidateSemantics::new("write", "Change a saved record", vec!["modify".into()])
                .unwrap(),
        ],
    };
    let request = NativeBlindAssessmentRequest::new(
        Uuid::new_v4(),
        run_id,
        1,
        fingerprint(&"reviewer").unwrap(),
        vec![row],
    )
    .unwrap();
    (database, request)
}

fn limits(requests: u32) -> ProviderLimits {
    ProviderLimits {
        maximum_requests: requests,
        maximum_input_tokens: 300,
        maximum_output_tokens: 150,
        maximum_cost_microusd: 30,
    }
}

fn reservation(
    request: &NativeBlindAssessmentRequest,
    attempt: u32,
) -> NativeReviewCallReservation {
    NativeReviewCallReservation {
        id: Uuid::new_v4(),
        category: NativeReviewOperationCategory::BlindSemanticAssessment,
        request: NativeReviewRequest::Blind(request.clone()),
        attempt,
        input_token_ceiling: 100,
        output_token_ceiling: 50,
        cost_ceiling_microusd: 10,
    }
}

fn success(
    request: &NativeBlindAssessmentRequest,
    reservation: NativeReviewCallReservation,
) -> NativeReviewCallOutcome {
    let evidence = NativeBlindAssessmentEvidence::record(
        request,
        NativeBlindAssessmentDraft {
            row_id: request.rows[0].row_id.clone(),
            row_fingerprint: request.rows[0].row_fingerprint.clone(),
            request_fingerprint: request.fingerprint.clone(),
            supported_candidate_ids: vec!["read".into()],
            ambiguous: false,
            context_consistent: true,
            issue_codes: vec![],
            rationale: "The question asks to retrieve a saved record.".into(),
        },
    )
    .unwrap();
    NativeReviewCallOutcome {
        reservation,
        usage: NativeReviewUsage {
            input_tokens: Some(20),
            output_tokens: Some(10),
            cost_microusd: Some(2),
        },
        response: Some(NativeReviewResponse::Blind(vec![evidence])),
        failure: None,
        interrupted: false,
    }
}

#[tokio::test]
async fn successful_review_is_immutable_and_cannot_be_dispatched_again() {
    let (mut database, request) = fixture().await;
    let call = reservation(&request, 1);
    reserve(&mut database, &call, &limits(4)).await.unwrap();
    let outcome = success(&request, call.clone());
    insert_outcome(&mut database, &outcome).await.unwrap();
    insert_outcome(&mut database, &outcome).await.unwrap();

    let history = read_calls(&mut database, request.run_id).await.unwrap();
    assert_eq!(history, vec![(call, Some(outcome))]);
    let error = reserve(&mut database, &reservation(&request, 2), &limits(4))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("interrupted first attempt"), "{error}");
    assert!(
        sqlx::query("DELETE FROM optimization_native_review_outcomes")
            .execute(&mut database)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn exactly_one_fresh_attempt_is_allowed_after_interrupted_unknown_outcome() {
    let (mut database, request) = fixture().await;
    let first = reservation(&request, 1);
    reserve(&mut database, &first, &limits(2)).await.unwrap();
    insert_outcome(
        &mut database,
        &NativeReviewCallOutcome {
            reservation: first,
            usage: NativeReviewUsage::default(),
            response: None,
            failure: None,
            interrupted: true,
        },
    )
    .await
    .unwrap();
    let second = reservation(&request, 2);
    reserve(&mut database, &second, &limits(2)).await.unwrap();
    insert_outcome(&mut database, &success(&request, second))
        .await
        .unwrap();

    let other = NativeBlindAssessmentRequest::new(
        Uuid::new_v4(),
        request.run_id,
        request.iteration,
        request.evaluator_identity_fingerprint.clone(),
        request.rows.clone(),
    )
    .unwrap();
    let error = reserve(&mut database, &reservation(&other, 1), &limits(2))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("request budget exhausted"), "{error}");
    let usage = budget_usage(&mut database, request.run_id).await.unwrap();
    assert_eq!(usage, (2, 0, 120, 60, 12));
}
