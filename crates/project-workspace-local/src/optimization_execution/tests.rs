use super::*;

#[tokio::test]
async fn stop_migration_preserves_original_attempt_bytes_and_immutability() {
    let mut db = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql(
        "PRAGMA foreign_keys=ON; CREATE TABLE project_optimization_runs(id TEXT PRIMARY KEY);",
    )
    .execute(&mut db)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/0019_optimization_agent_execution.sql"
    ))
    .execute(&mut db)
    .await
    .unwrap();
    let run = Uuid::new_v4();
    sqlx::query("INSERT INTO project_optimization_runs(id) VALUES(?)")
        .bind(run.to_string())
        .execute(&mut db)
        .await
        .unwrap();
    let event = AgentExecutionEvent::create(
        project_workspace_core::BoundIdentity {
            id: run.to_string(),
            fingerprint: format!("sha256:{}", "a".repeat(64)),
        },
        None,
        Uuid::new_v4(),
        AgentExecutionChange::Started,
        Utc::now(),
    )
    .unwrap();
    let bytes = serde_json::to_string(&event).unwrap();
    sqlx::query("INSERT INTO optimization_agent_execution_events(id,run_id,sequence,attempt_id,kind,fingerprint,metadata_json) VALUES(?,?,1,?,'started',?,?)")
        .bind(event.id.to_string()).bind(run.to_string()).bind(event.attempt_id.to_string()).bind(&event.fingerprint).bind(&bytes).execute(&mut db).await.unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/0020_optimization_agent_stop.sql"
    ))
    .execute(&mut db)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/0022_optimization_agent_budget_stop.sql"
    ))
    .execute(&mut db)
    .await
    .unwrap();
    assert_eq!(read(&mut db, run).await.unwrap(), vec![event.clone()]);
    let saved: String =
        sqlx::query_scalar("SELECT metadata_json FROM optimization_agent_execution_events")
            .fetch_one(&mut db)
            .await
            .unwrap();
    assert_eq!(saved, bytes);
    let exhausted = AgentExecutionEvent::create(
        project_workspace_core::BoundIdentity {
            id: run.to_string(),
            fingerprint: format!("sha256:{}", "a".repeat(64)),
        },
        Some(&event),
        event.attempt_id,
        AgentExecutionChange::BudgetExhausted,
        Utc::now(),
    )
    .unwrap();
    sqlx::query("INSERT INTO optimization_agent_execution_events(id,run_id,sequence,attempt_id,kind,fingerprint,metadata_json) VALUES(?,?,2,?,'budget_exhausted',?,?)")
        .bind(exhausted.id.to_string()).bind(run.to_string()).bind(exhausted.attempt_id.to_string()).bind(&exhausted.fingerprint).bind(serde_json::to_string(&exhausted).unwrap()).execute(&mut db).await.unwrap();
    assert_eq!(read(&mut db, run).await.unwrap(), vec![event, exhausted]);
    assert!(
        sqlx::query("UPDATE optimization_agent_execution_events SET kind='paused'")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM optimization_agent_execution_events")
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
}
