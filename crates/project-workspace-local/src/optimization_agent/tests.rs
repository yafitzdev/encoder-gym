use super::*;

async fn fixture() -> (SqliteConnection, AgentAnalysisScope, ProviderLimits) {
    let mut database = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    // Minimal parent contract for these adapter unit tests. Ordinary workspace
    // integration tests also apply this migration after all twelve predecessors.
    sqlx::raw_sql("PRAGMA foreign_keys=ON; CREATE TABLE project_optimization_runs(id TEXT PRIMARY KEY); CREATE TABLE project_optimization_events(run_id TEXT, sequence INTEGER, kind TEXT);").execute(&mut database).await.unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0013_optimization_agent.sql"))
        .execute(&mut database)
        .await
        .unwrap();
    let scope = AgentAnalysisScope {
        run_id: Uuid::new_v4(),
        iteration: 1,
        launch_fingerprint: fingerprint(&"launch").unwrap(),
        dataset_version_id: Uuid::new_v4(),
        dataset_fingerprint: fingerprint(&"members").unwrap(),
        development_evidence_fingerprint: fingerprint(&"development").unwrap(),
        objective: String::new(),
        maximum_turns: 4,
        maximum_row_changes: 8,
    };
    sqlx::query("INSERT INTO project_optimization_runs VALUES (?)")
        .bind(scope.run_id.to_string())
        .execute(&mut database)
        .await
        .unwrap();
    (
        database,
        scope,
        ProviderLimits {
            maximum_requests: 4,
            maximum_input_tokens: 300,
            maximum_output_tokens: 150,
            maximum_cost_microusd: 30,
        },
    )
}

fn call(scope: &AgentAnalysisScope, sequence: u32) -> AgentCallReservation {
    AgentCallReservation {
        id: Uuid::new_v4(),
        scope_fingerprint: scope.fingerprint().unwrap(),
        sequence,
        request_fingerprint: fingerprint(&sequence).unwrap(),
        input_token_ceiling: 100,
        output_token_ceiling: 50,
        cost_ceiling_microusd: 10,
    }
}

fn outcome(call: AgentCallReservation, interrupted: bool) -> AgentTurnRecord {
    AgentTurnRecord {
        call,
        explanations: Vec::new(),
        tools: Vec::new(),
        usage: if interrupted {
            AgentTokenUsage::default()
        } else {
            AgentTokenUsage {
                input_tokens: Some(20),
                output_tokens: Some(10),
                cost_microusd: Some(2),
            }
        },
        proposal: None,
        interrupted,
    }
}

#[tokio::test]
async fn replay_rejects_self_consistent_json_that_disagrees_with_normalized_identity() {
    let (mut db, scope, limits) = fixture().await;
    let mut request = call(&scope, 1);
    reserve(&mut db, &scope, &request, &limits).await.unwrap();
    sqlx::query("DROP TRIGGER immutable_optimization_agent_calls_update")
        .execute(&mut db)
        .await
        .unwrap();
    request.id = Uuid::new_v4();
    sqlx::query("UPDATE optimization_agent_calls SET metadata_json=?, fingerprint=?")
        .bind(serde_json::to_string(&request).unwrap())
        .bind(fingerprint(&request).unwrap())
        .execute(&mut db)
        .await
        .unwrap();
    assert!(
        read_history(&mut db, scope.run_id, None)
            .await
            .unwrap_err()
            .to_string()
            .contains("reservation was modified")
    );
}

#[tokio::test]
async fn reservations_are_exclusive_and_outcomes_are_append_only_and_reloaded() {
    let (mut db, scope, limits) = fixture().await;
    let first = call(&scope, 1);
    reserve(&mut db, &scope, &first, &limits).await.unwrap();
    assert!(
        reserve(&mut db, &scope, &call(&scope, 2), &limits)
            .await
            .unwrap_err()
            .to_string()
            .contains("pending")
    );
    let first = outcome(first, false);
    insert_outcome(&mut db, &first).await.unwrap();
    let history = read_history(&mut db, scope.run_id, None).await.unwrap();
    assert_eq!(history[0].1.as_ref(), Some(&first));
    assert!(insert_outcome(&mut db, &first).await.is_err());
    assert!(
        sqlx::query("UPDATE optimization_agent_calls SET metadata_json='{}'")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM optimization_agent_outcomes")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM optimization_agent_scopes")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&mut db)
            .await
            .unwrap()
            .is_empty()
    );
    reserve(&mut db, &scope, &call(&scope, 2), &limits)
        .await
        .unwrap();
}

#[tokio::test]
async fn unknown_outcomes_consume_reserved_tokens_and_cost_across_iterations() {
    let (mut db, scope, limits) = fixture().await;
    for sequence in 1..=3 {
        let call = call(&scope, sequence);
        reserve(&mut db, &scope, &call, &limits).await.unwrap();
        insert_outcome(&mut db, &outcome(call, true)).await.unwrap();
    }
    let mut next = scope.clone();
    next.iteration = 2;
    assert!(
        reserve(&mut db, &next, &call(&next, 1), &limits)
            .await
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    assert_eq!(
        read_history(&mut db, scope.run_id, None)
            .await
            .unwrap()
            .len(),
        3
    );
}

#[tokio::test]
async fn reported_usage_releases_unused_reservation_but_request_limit_remains() {
    let (mut db, scope, mut limits) = fixture().await;
    limits.maximum_requests = 2;
    for sequence in 1..=2 {
        let call = call(&scope, sequence);
        reserve(&mut db, &scope, &call, &limits).await.unwrap();
        insert_outcome(&mut db, &outcome(call, false))
            .await
            .unwrap();
    }
    assert!(
        reserve(&mut db, &scope, &call(&scope, 3), &limits)
            .await
            .unwrap_err()
            .to_string()
            .contains("request budget")
    );
}

#[tokio::test]
async fn zero_reported_catalog_cost_is_not_substituted_for_unknown_cost() {
    let (mut db, scope, mut limits) = fixture().await;
    limits.maximum_cost_microusd = 10;
    let first = call(&scope, 1);
    reserve(&mut db, &scope, &first, &limits).await.unwrap();
    let mut record = outcome(first, false);
    record.usage.cost_microusd = None;
    insert_outcome(&mut db, &record).await.unwrap();
    let error = reserve(&mut db, &scope, &call(&scope, 2), &limits)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("spend budget exhausted"), "{error}");
    assert!(error.contains("projected 20"), "{error}");
    assert!(error.contains("limit 10 microusd"), "{error}");
}

#[tokio::test]
async fn reservation_reports_the_exact_exhausted_token_dimension() {
    let (mut db, scope, mut limits) = fixture().await;
    limits.maximum_input_tokens = 99;
    let error = reserve(&mut db, &scope, &call(&scope, 1), &limits)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("input token budget exhausted"), "{error}");
    assert!(error.contains("projected 100"), "{error}");
    assert!(error.contains("limit 99"), "{error}");

    let (mut db, scope, mut limits) = fixture().await;
    limits.maximum_output_tokens = 49;
    let error = reserve(&mut db, &scope, &call(&scope, 1), &limits)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("output token budget exhausted"), "{error}");
    assert!(error.contains("projected 50"), "{error}");
    assert!(error.contains("limit 49"), "{error}");
}

#[tokio::test]
async fn cancellation_wins_before_reservation_and_scope_changes_cannot_replace_history() {
    let (mut db, scope, limits) = fixture().await;
    let first = call(&scope, 1);
    reserve(&mut db, &scope, &first, &limits).await.unwrap();
    insert_outcome(&mut db, &outcome(first, false))
        .await
        .unwrap();
    let mut changed = scope.clone();
    changed.dataset_fingerprint = fingerprint(&"different rows").unwrap();
    assert!(
        reserve(&mut db, &changed, &call(&changed, 1), &limits)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO project_optimization_events VALUES (?,1,'cancelled')")
        .bind(scope.run_id.to_string())
        .execute(&mut db)
        .await
        .unwrap();
    assert!(
        reserve(&mut db, &scope, &call(&scope, 2), &limits)
            .await
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
}
