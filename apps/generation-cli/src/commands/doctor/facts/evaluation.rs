use super::super::*;

pub(in crate::commands::doctor) async fn evaluation_facts_check(
    store: &SqliteStore,
) -> DoctorCheck {
    let runs = match store.list_evaluation_runs().await {
        Ok(runs) => runs,
        Err(error) => return fail("evaluation_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for run in &runs {
        let count = match store.count_predictions(run.id).await {
            Ok(count) => count,
            Err(error) => {
                failures.push(format!("{}: {error}", run.id));
                continue;
            }
        };
        if run.state == EvaluationRunState::Completed
            && (count != run.total_examples || run.example_count != count || run.metrics.is_none())
        {
            failures.push(format!(
                "{}: completed run has inconsistent count or metrics",
                run.id
            ));
        }
    }
    let duplicate_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM (SELECT evaluation_run_id, snapshot_member_id FROM evaluation_predictions GROUP BY evaluation_run_id, snapshot_member_id HAVING COUNT(*) > 1)")
        .fetch_one(store.pool()).await.unwrap_or(-1);
    if duplicate_count != 0 {
        failures.push(format!("duplicate prediction groups: {duplicate_count}"));
    }
    let invalid_comparisons = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM evaluation_comparisons c JOIN evaluation_runs l ON l.id = c.left_run_id JOIN evaluation_runs r ON r.id = c.right_run_id WHERE l.state <> 'completed' OR r.state <> 'completed' OR l.snapshot_id <> r.snapshot_id OR l.split <> r.split OR l.protocol_fingerprint <> r.protocol_fingerprint")
        .fetch_one(store.pool()).await.unwrap_or(-1);
    if invalid_comparisons != 0 {
        failures.push(format!("invalid comparison reports: {invalid_comparisons}"));
    }
    match store.list_comparisons(u32::MAX, 0).await {
        Ok(reports) => {
            for report in reports {
                if comparison_fingerprint(&report).ok().as_deref() != Some(&report.fingerprint) {
                    failures.push(format!("{}: comparison fingerprint mismatch", report.id));
                }
            }
        }
        Err(error) => failures.push(error.to_string()),
    }
    match store.list_selections(u32::MAX, 0).await {
        Ok(reports) => {
            for report in reports {
                if selection_fingerprint(&report).ok().as_deref() != Some(&report.fingerprint) {
                    failures.push(format!("{}: selection fingerprint mismatch", report.id));
                }
                for candidate in &report.candidate_run_ids {
                    if !runs.iter().any(|run| {
                        run.id == *candidate && run.state == EvaluationRunState::Completed
                    }) {
                        failures.push(format!(
                            "{}: incomplete or missing candidate {candidate}",
                            report.id
                        ));
                    }
                }
            }
        }
        Err(error) => failures.push(error.to_string()),
    }
    if failures.is_empty() {
        pass(
            "evaluation_facts",
            format!("{} evaluation run(s) verified", runs.len()),
        )
    } else {
        fail("evaluation_facts", failures.join("; "))
    }
}
