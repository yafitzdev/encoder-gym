use super::super::*;

pub(in crate::commands::doctor) async fn analysis_facts_check(store: &SqliteStore) -> DoctorCheck {
    let reports = match store.list_analysis_reports().await {
        Ok(reports) => reports,
        Err(error) => return fail("analysis_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    for report in &reports {
        let legacy = report.protocol_fingerprint.is_empty()
            || report
                .source_identity
                .as_ref()
                .is_none_or(|source| source.evaluation_input_fingerprint.starts_with("legacy:"));
        if legacy {
            continue;
        }
        if let Err(error) = verify_report_evidence(store, report).await {
            failures.push(format!("{}: {error}", report.id));
        }
        if report.protocol.validate().is_err()
            || report.protocol.fingerprint().ok().as_deref()
                != Some(report.protocol_fingerprint.as_str())
        {
            failures.push(format!("{}: protocol fingerprint mismatch", report.id));
        }
        if reproduce_report_fingerprint(report).ok().as_deref() != Some(report.fingerprint.as_str())
        {
            failures.push(format!("{}: report fingerprint mismatch", report.id));
        }
        let Some(source) = &report.source_identity else {
            failures.push(format!("{}: source identity missing", report.id));
            continue;
        };
        match store.get_evaluation_run(report.evaluation_run_id).await {
            Ok(Some(run)) => {
                let count = store.count_predictions(run.id).await.unwrap_or(u64::MAX);
                if run.state != EvaluationRunState::Completed
                    || count != report.prediction_count
                    || source.prediction_count != count
                    || source.evaluation_input_fingerprint != run.input_fingerprint
                    || source.evaluation_protocol_fingerprint != run.protocol_fingerprint
                    || source.cohort_fingerprint != run.source_identity.cohort_fingerprint
                {
                    failures.push(format!(
                        "{}: evaluation source identity mismatch",
                        report.id
                    ));
                }
            }
            Ok(None) => failures.push(format!("{}: evaluation run missing", report.id)),
            Err(error) => failures.push(format!("{}: {error}", report.id)),
        }
        if let Some(comparison_id) = source.comparison_id {
            match store.get_comparison(comparison_id).await {
                Ok(Some(comparison))
                    if source.comparison_fingerprint.as_deref()
                        == Some(comparison.fingerprint.as_str())
                        && (comparison.left_run_id == report.evaluation_run_id
                            || comparison.right_run_id == report.evaluation_run_id) => {}
                Ok(Some(_)) => failures.push(format!(
                    "{}: comparison source identity mismatch",
                    report.id
                )),
                Ok(None) => failures.push(format!("{}: comparison missing", report.id)),
                Err(error) => failures.push(format!("{}: {error}", report.id)),
            }
        }
        let mut keys = std::collections::BTreeSet::new();
        let mut previous_cumulative = 0_u64;
        for (index, finding) in report.findings.iter().enumerate() {
            let identity = FindingIdentity {
                kind: finding.kind,
                attributes: finding.attributes.clone(),
            };
            if finding.rank != index as u64 + 1
                || identity.key() != finding.key
                || finding.reproduce_fingerprint().ok().as_deref()
                    != Some(finding.fingerprint.as_str())
                || !keys.insert(&finding.key)
                || finding.error_count > finding.support
                || finding.cumulative_error_count
                    != previous_cumulative.saturating_add(finding.marginal_error_count)
                || finding.cumulative_error_count > report.error_count
            {
                failures.push(format!("{}: invalid finding {}", report.id, finding.key));
            }
            previous_cumulative = finding.cumulative_error_count;
            let normalized_evidence = store
                .query_finding_evidence(FindingEvidenceQuery {
                    analysis_report_id: report.id,
                    finding_key: finding.key.clone(),
                    category: None,
                    limit: 1_000,
                    offset: 0,
                })
                .await;
            let expected_evidence = report
                .finding_evidence
                .get(&finding.key)
                .map(Vec::as_slice)
                .unwrap_or_default();
            if normalized_evidence.as_deref() != Ok(expected_evidence) {
                failures.push(format!(
                    "{}: normalized evidence differs for {}",
                    report.id, finding.key
                ));
            }
        }
        let normalized = store
            .query_analysis_findings(AnalysisFindingQuery {
                analysis_report_id: report.id,
                kind: None,
                minimum_support: None,
                minimum_error_count: None,
                sort: Default::default(),
                limit: u32::MAX,
                offset: 0,
            })
            .await;
        if normalized.as_ref().ok() != Some(&report.findings) {
            failures.push(format!("{}: normalized findings differ", report.id));
        }
        let invalid_evidence: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM analysis_finding_evidence e \
             JOIN analysis_reports a ON a.id = e.analysis_report_id \
             JOIN evaluation_predictions p ON p.id = e.prediction_id \
             WHERE e.analysis_report_id = ? AND (p.evaluation_run_id <> a.evaluation_run_id \
             OR p.snapshot_member_id <> e.snapshot_member_id \
             OR p.source_row_id <> e.source_row_id)",
        )
        .bind(report.id)
        .fetch_one(store.pool())
        .await
        .unwrap_or(-1);
        if invalid_evidence != 0 {
            failures.push(format!("{}: invalid evidence links", report.id));
        }
    }
    let orphan_reviews: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM analysis_finding_reviews r \
         LEFT JOIN analysis_findings f ON f.analysis_report_id = r.analysis_report_id \
         AND f.finding_key = r.finding_key WHERE f.finding_key IS NULL",
    )
    .fetch_one(store.pool())
    .await
    .unwrap_or(-1);
    if orphan_reviews != 0 {
        failures.push(format!("orphan finding reviews: {orphan_reviews}"));
    }
    if failures.is_empty() {
        pass(
            "analysis_facts",
            format!("{} analysis report(s) verified", reports.len()),
        )
    } else {
        fail("analysis_facts", failures.join("; "))
    }
}
