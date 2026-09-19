use super::*;
use encoder_optimization_core::{
    agent::{
        AgentAnalysisScope, AgentCallReservation, AgentTurnRecord, DatasetEditProposal,
        GenerationTarget, RecordedAgentTool,
    },
    generation::{GenerationAdmission, RejectedGenerationRow},
};
use serde_json::json;

async fn fixture() -> (SqliteConnection, GenerationTask, ProviderLimits) {
    let mut database = SqliteConnection::connect("sqlite::memory:").await.unwrap();
    sqlx::raw_sql("PRAGMA foreign_keys=ON; CREATE TABLE project_optimization_runs(id TEXT PRIMARY KEY); CREATE TABLE project_optimization_events(run_id TEXT, sequence INTEGER, kind TEXT);").execute(&mut database).await.unwrap();
    sqlx::raw_sql(include_str!("../../migrations/0013_optimization_agent.sql"))
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../migrations/0014_optimization_generation.sql"
    ))
    .execute(&mut database)
    .await
    .unwrap();
    let scope = AgentAnalysisScope {
        run_id: Uuid::new_v4(),
        iteration: 1,
        launch_fingerprint: fingerprint(&"launch").unwrap(),
        dataset_version_id: Uuid::new_v4(),
        dataset_fingerprint: fingerprint(&"dataset").unwrap(),
        development_evidence_fingerprint: fingerprint(&"evidence").unwrap(),
        objective: String::new(),
        analysis_protocol: 1,
        maximum_turns: 4,
        maximum_row_changes: 16,
    };
    let proposal = DatasetEditProposal {
        summary: "Generate two bounded batches".into(),
        stop: false,
        stop_reason: None,
        removals: vec![],
        additions: vec![GenerationTarget {
            template_row_id: "member".into(),
            instruction: "Fill a diagnosed gap".into(),
            count: 16,
            evidence_ids: vec!["failure".into()],
        }],
    };
    let call = AgentCallReservation {
        id: Uuid::new_v4(),
        scope_fingerprint: scope.fingerprint().unwrap(),
        sequence: 1,
        request_fingerprint: fingerprint(&"prompt").unwrap(),
        input_token_ceiling: 100,
        output_token_ceiling: 50,
        cost_ceiling_microusd: 10,
    };
    let record = AgentTurnRecord {
        call: call.clone(),
        explanations: vec![proposal.summary.clone()],
        tools: vec![RecordedAgentTool {
            call_id: "tool-1".into(),
            name: "propose_dataset_edits".into(),
            arguments: serde_json::to_value(&proposal).unwrap(),
            result: json!({"accepted":true}),
            failed: false,
        }],
        usage: AgentTokenUsage::default(),
        proposal: Some(proposal.clone()),
        interrupted: false,
    };
    record.validate().unwrap();
    sqlx::query("INSERT INTO project_optimization_runs VALUES(?)")
        .bind(scope.run_id.to_string())
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO optimization_agent_scopes VALUES(?,?,?,?)")
        .bind(scope.run_id.to_string())
        .bind(1_i64)
        .bind(scope.fingerprint().unwrap())
        .bind(serde_json::to_string(&scope).unwrap())
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO optimization_agent_calls VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(call.id.to_string())
        .bind(scope.run_id.to_string())
        .bind(1_i64)
        .bind(1_i64)
        .bind(&call.scope_fingerprint)
        .bind(fingerprint(&call).unwrap())
        .bind(serde_json::to_string(&call).unwrap())
        .bind(0_i64)
        .bind(0_i64)
        .bind("fixture")
        .execute(&mut database)
        .await
        .unwrap();
    sqlx::query("INSERT INTO optimization_agent_outcomes VALUES(?,?,?,?)")
        .bind(call.id.to_string())
        .bind(fingerprint(&record).unwrap())
        .bind(serde_json::to_string(&record).unwrap())
        .bind("fixture")
        .execute(&mut database)
        .await
        .unwrap();
    let request = serde_json::from_value(
        json!({"systemPrompt":"generate","userPrompt":"a row","maximumOutputTokens":50}),
    )
    .unwrap();
    let task = GenerationTask {
        id: Uuid::new_v4(),
        run_id: scope.run_id,
        iteration: 1,
        proposal_fingerprint: fingerprint(&proposal).unwrap(),
        template_row_id: "member".into(),
        template_fingerprint: fingerprint(&"template").unwrap(),
        target_index: 0,
        first_row: 0,
        requested_rows: 8,
        request,
        execution_v3: None,
    };
    (
        database,
        task,
        ProviderLimits {
            maximum_requests: 4,
            maximum_input_tokens: 300,
            maximum_output_tokens: 150,
            maximum_cost_microusd: 30,
        },
    )
}

fn call(task: &GenerationTask, attempt: u32) -> GenerationReservation {
    GenerationReservation {
        id: Uuid::new_v4(),
        task_id: task.id,
        task_fingerprint: task.fingerprint().unwrap(),
        attempt,
        input_token_ceiling: 100,
        output_token_ceiling: 50,
        cost_ceiling_microusd: 10,
    }
}
fn outcome(
    task: &GenerationTask,
    call: GenerationReservation,
    interrupted: bool,
) -> GenerationOutcome {
    GenerationOutcome {
        reservation: call,
        usage: if interrupted {
            AgentTokenUsage::default()
        } else {
            AgentTokenUsage {
                input_tokens: Some(20),
                output_tokens: Some(10),
                cost_microusd: None,
            }
        },
        admission: (!interrupted).then(|| GenerationAdmission {
            accepted: vec![],
            rejected: (0..task.requested_rows)
                .map(|index| RejectedGenerationRow {
                    index,
                    reason: "invalid task output".into(),
                })
                .collect(),
        }),
        interrupted,
    }
}

#[tokio::test]
async fn pending_calls_obey_concurrency_and_completed_rejections_are_not_retried() {
    let (mut db, task, limits) = fixture().await;
    let first = call(&task, 1);
    reserve(&mut db, &task, &first, &limits, 1).await.unwrap();
    let mut next = task.clone();
    next.id = Uuid::new_v4();
    next.first_row = 8;
    assert!(
        reserve(&mut db, &next, &call(&next, 1), &limits, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("concurrency")
    );
    reserve(&mut db, &next, &call(&next, 1), &limits, 2)
        .await
        .unwrap();
    let record = outcome(&task, first, false);
    insert_outcome(&mut db, &task, &record).await.unwrap();
    insert_outcome(&mut db, &task, &record).await.unwrap();
    assert!(
        reserve(&mut db, &task, &call(&task, 2), &limits, 2)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM optimization_generation_outcomes")
            .execute(&mut db)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE optimization_generation_tasks SET metadata_json='{}'")
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

#[tokio::test]
async fn unknown_attempts_consume_budget_and_resume_cannot_substitute_inputs() {
    let (mut db, task, limits) = fixture().await;
    for attempt in 1..=3 {
        let call = call(&task, attempt);
        reserve(&mut db, &task, &call, &limits, 1).await.unwrap();
        insert_outcome(&mut db, &task, &outcome(&task, call, true))
            .await
            .unwrap();
    }
    assert!(
        reserve(&mut db, &task, &call(&task, 4), &limits, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    let mut changed = task.clone();
    changed.request.user_prompt = "new prompt".into();
    assert!(
        reserve(&mut db, &changed, &call(&changed, 4), &limits, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
}

#[tokio::test]
async fn calls_require_exact_recorded_proposal_and_stop_wins_before_dispatch() {
    let (mut db, task, limits) = fixture().await;
    let mut foreign = task.clone();
    foreign.proposal_fingerprint = fingerprint(&"invented").unwrap();
    assert!(
        reserve(&mut db, &foreign, &call(&foreign, 1), &limits, 1)
            .await
            .is_err()
    );
    let mut extra = task.clone();
    extra.first_row = 4;
    assert!(
        reserve(&mut db, &extra, &call(&extra, 1), &limits, 1)
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO project_optimization_events VALUES(?,1,'cancelled')")
        .bind(task.run_id.to_string())
        .execute(&mut db)
        .await
        .unwrap();
    assert!(
        reserve(&mut db, &task, &call(&task, 1), &limits, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("stopped")
    );
    assert!(read_calls(&mut db, task.run_id).await.unwrap().is_empty());
}

#[tokio::test]
async fn reported_overrun_is_retained_and_charged_even_when_admission_is_interrupted() {
    let (mut db, task, limits) = fixture().await;
    let first = call(&task, 1);
    reserve(&mut db, &task, &first, &limits, 1).await.unwrap();
    let mut record = outcome(&task, first, true);
    record.usage.input_tokens = Some(350);
    insert_outcome(&mut db, &task, &record).await.unwrap();
    assert!(
        reserve(&mut db, &task, &call(&task, 2), &limits, 1)
            .await
            .unwrap_err()
            .to_string()
            .contains("budget")
    );
    assert_eq!(
        read_calls(&mut db, task.run_id).await.unwrap()[0]
            .2
            .as_ref()
            .unwrap()
            .usage
            .input_tokens,
        Some(350)
    );
}
