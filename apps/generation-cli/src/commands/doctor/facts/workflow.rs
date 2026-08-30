use super::super::*;

pub(in crate::commands::doctor) async fn workflow_facts_check(store: &SqliteStore) -> DoctorCheck {
    let definitions = match store
        .query_workflow_definitions(WorkflowDefinitionQuery {
            dataset_id: None,
            limit: 10_000,
            offset: 0,
        })
        .await
    {
        Ok(values) => values,
        Err(error) => return fail("workflow_facts", error.to_string()),
    };
    let runs = match store
        .query_workflow_runs(WorkflowRunQuery {
            definition_id: None,
            state: None,
            limit: 10_000,
            offset: 0,
        })
        .await
    {
        Ok(values) => values,
        Err(error) => return fail("workflow_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for definition in &definitions {
        if definition.reproduce_fingerprint().ok().as_deref()
            != Some(definition.fingerprint.as_str())
        {
            failures.push(format!("definition {} fingerprint mismatch", definition.id));
        }
    }
    for run in &runs {
        let Some(definition) = definitions
            .iter()
            .find(|definition| definition.id == run.definition_id)
        else {
            failures.push(format!("run {} definition is missing", run.id));
            continue;
        };
        if run.definition_fingerprint != definition.fingerprint {
            failures.push(format!("run {} definition fingerprint mismatch", run.id));
        }
        let attempts = match store.list_workflow_attempts(run.id).await {
            Ok(values) => values,
            Err(error) => {
                failures.push(format!("run {}: {error}", run.id));
                continue;
            }
        };
        for (index, attempt) in attempts.iter().enumerate() {
            if attempt.reproduce_fingerprint().ok().as_deref() != Some(attempt.fingerprint.as_str())
                || usize::try_from(attempt.sequence).ok() != Some(index)
                || index > 0
                    && (attempt.predecessor_id != Some(attempts[index - 1].id)
                        || attempt.predecessor_fingerprint.as_deref()
                            != Some(attempts[index - 1].fingerprint.as_str()))
            {
                failures.push(format!("run {} attempt chain is invalid", run.id));
                break;
            }
        }
        if attempts.last().map(|attempt| attempt.id) != run.latest_attempt_id
            || attempts.last().map(|attempt| attempt.fingerprint.as_str())
                != run.latest_attempt_fingerprint.as_deref()
        {
            failures.push(format!("run {} latest attempt projection mismatch", run.id));
        }
        if run.usage.validate_against(&definition.budget).is_err() {
            failures.push(format!("run {} exceeds its resolved budget", run.id));
        }
        let normalized_links = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM workflow_artifact_links links \
             JOIN workflow_stage_attempts attempts \
               ON attempts.id = links.workflow_stage_attempt_id \
             WHERE attempts.workflow_run_id = ?",
        )
        .bind(run.id)
        .fetch_one(store.pool())
        .await
        .unwrap_or(-1);
        let expected_links = attempts
            .iter()
            .map(|attempt| attempt.artifacts.len())
            .sum::<usize>();
        if usize::try_from(normalized_links).ok() != Some(expected_links) {
            failures.push(format!("run {} normalized artifact links differ", run.id));
        }
    }
    let artifact_checks = [
        verify_artifact_json::<InitialAllocationRecord, _>(
            store,
            "workflow_initial_allocations",
            |value| {
                value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
                    && value.result.reproduce_fingerprint().ok().as_deref()
                        == Some(&value.result.fingerprint)
            },
        )
        .await,
        verify_artifact_json::<BenchmarkSuite, _>(store, "workflow_benchmark_suites", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
        verify_artifact_json::<AcceptanceAssessment, _>(
            store,
            "workflow_acceptance_assessments",
            |value| value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint),
        )
        .await,
        verify_artifact_json::<EvidenceExposure, _>(
            store,
            "workflow_evidence_exposures",
            |value| value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint),
        )
        .await,
        verify_artifact_json::<AdvisoryAssessment, _>(store, "advisory_assessments", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
        verify_artifact_json::<WorkflowApprovalDecision, _>(
            store,
            "workflow_approval_decisions",
            |value| value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint),
        )
        .await,
        verify_artifact_json::<StopDecision, _>(store, "workflow_stop_decisions", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
        verify_artifact_json::<ModelPromotion, _>(store, "model_promotions", |value| {
            value.reproduce_fingerprint().ok().as_deref() == Some(&value.fingerprint)
        })
        .await,
    ];
    let mut checked_artifacts = 0;
    for result in artifact_checks {
        match result {
            Ok(count) => checked_artifacts += count,
            Err(error) => failures.push(error),
        }
    }
    if failures.is_empty() {
        pass(
            "workflow_facts",
            format!(
                "{} definition(s), {} run chain(s), and {} workflow artifact(s) verified",
                definitions.len(),
                runs.len(),
                checked_artifacts,
            ),
        )
    } else {
        fail("workflow_facts", failures.join("; "))
    }
}

async fn verify_artifact_json<T, F>(
    store: &SqliteStore,
    table: &'static str,
    verify: F,
) -> Result<usize, String>
where
    T: DeserializeOwned,
    F: Fn(&T) -> bool,
{
    let rows = sqlx::query_scalar::<_, String>(&format!("SELECT artifact_json FROM {table}"))
        .fetch_all(store.pool())
        .await
        .map_err(|error| format!("{table}: {error}"))?;
    for (index, json) in rows.iter().enumerate() {
        let artifact = serde_json::from_str::<T>(json)
            .map_err(|error| format!("{table} row {index}: {error}"))?;
        if !verify(&artifact) {
            return Err(format!("{table} row {index}: fingerprint mismatch"));
        }
    }
    Ok(rows.len())
}
