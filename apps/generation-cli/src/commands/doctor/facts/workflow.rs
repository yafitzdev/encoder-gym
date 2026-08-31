use super::super::*;

pub(in crate::commands::doctor) async fn bootstrap_facts_check(store: &SqliteStore) -> DoctorCheck {
    let bootstraps = match store.list_bootstraps(10_000, 0).await {
        Ok(values) => values,
        Err(error) => return fail("project_bootstrap_facts", error.to_string()),
    };
    let mut failures = Vec::new();
    let mut source_count = 0_usize;
    for bootstrap in &bootstraps {
        if bootstrap.reproduce_fingerprint().ok().as_deref() != Some(bootstrap.fingerprint.as_str())
        {
            failures.push(format!("bootstrap {} fingerprint mismatch", bootstrap.id));
            continue;
        }
        match store.get_preparation(bootstrap.preparation_id).await {
            Ok(Some(_)) => {}
            Ok(None) => failures.push(format!(
                "bootstrap {} preparation is missing: {}",
                bootstrap.id, bootstrap.preparation_id
            )),
            Err(error) => failures.push(format!("bootstrap {}: {error}", bootstrap.id)),
        }
        for source in &bootstrap.sources {
            source_count += 1;
            let dataset = store.get_dataset(source.dataset_id).await;
            let dataset_import = store.get_import(source.import_id).await;
            let snapshot = store.get_snapshot(source.snapshot_id).await;
            let members = store.list_snapshot_members(source.snapshot_id).await;
            match (dataset, dataset_import, snapshot, members) {
                (
                    Ok(Some(dataset)),
                    Ok(Some(dataset_import)),
                    Ok(Some(snapshot)),
                    Ok(members),
                ) if dataset_import.dataset_id == dataset.id
                    && dataset_import.state == ImportState::Completed
                    && dataset_import.accepted_rows == source.accepted_rows
                    && snapshot.source_dataset_id == dataset.id
                    && snapshot.member_count == source.accepted_rows
                    && members.len() as u64 == source.accepted_rows
                    && members.iter().all(|member| {
                        member.split == SnapshotSplit::Test
                            && matches!(
                                member.source_provenance,
                                SourceProvenance::Imported { import_id, .. }
                                    if import_id == dataset_import.id
                            )
                    }) => {}
                (Ok(Some(_)), Ok(Some(_)), Ok(Some(_)), Ok(_)) => failures.push(format!(
                    "bootstrap {} source {} references or counts differ",
                    bootstrap.id, source.key
                )),
                (dataset, dataset_import, snapshot, members) => failures.push(format!(
                    "bootstrap {} source {} is incomplete (dataset={}, import={}, snapshot={}, members={})",
                    bootstrap.id,
                    source.key,
                    result_state(&dataset),
                    result_state(&dataset_import),
                    result_state(&snapshot),
                    if members.is_ok() { "ok" } else { "error" },
                )),
            }
        }
    }
    if failures.is_empty() {
        pass(
            "project_bootstrap_facts",
            format!(
                "{} bootstrap(s) and {source_count} immutable local source(s) verified",
                bootstraps.len()
            ),
        )
    } else {
        fail("project_bootstrap_facts", failures.join("; "))
    }
}

fn result_state<T, E>(result: &Result<Option<T>, E>) -> &'static str {
    match result {
        Ok(Some(_)) => "ok",
        Ok(None) => "missing",
        Err(_) => "error",
    }
}

pub(in crate::commands::doctor) async fn benchmark_bundle_facts_check(
    store: &SqliteStore,
) -> DoctorCheck {
    let result: anyhow::Result<(usize, usize, usize, usize)> = async {
        let bundles = store
            .query_benchmark_bundles(BenchmarkBundleQuery {
                development_suite_id: None,
                sealed_suite_id: None,
                contamination_report_id: None,
                limit: 10_000,
                offset: 0,
            })
            .await?;
        let bundles = bundles
            .iter()
            .map(|bundle| (bundle.id, bundle))
            .collect::<std::collections::BTreeMap<_, _>>();

        let definitions = store
            .query_workflow_definitions(WorkflowDefinitionQuery {
                dataset_id: None,
                limit: 10_000,
                offset: 0,
            })
            .await?;
        let definitions = definitions
            .iter()
            .map(|definition| (definition.id, definition))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut legacy_definitions = 0_usize;
        for definition in definitions.values() {
            let Some(binding) = &definition.benchmark_bundle else {
                legacy_definitions += 1;
                continue;
            };
            let bundle = bundles.get(&binding.bundle_id).with_context(|| {
                format!(
                    "workflow definition {} benchmark bundle is missing: {}",
                    definition.id, binding.bundle_id
                )
            })?;
            binding.validate_bundle(bundle).with_context(|| {
                format!(
                    "workflow definition {} benchmark bundle binding differs",
                    definition.id
                )
            })?;
        }

        let preparations = store.list_preparations(10_000, 0).await?;
        let mut legacy_preparations = 0_usize;
        for preparation in &preparations {
            let definition = definitions
                .get(&preparation.workflow_definition_id)
                .with_context(|| {
                    format!(
                        "preparation {} workflow definition is missing: {}",
                        preparation.id, preparation.workflow_definition_id
                    )
                })?;
            match (
                preparation.benchmark_bundle_id,
                preparation.benchmark_bundle_fingerprint.as_deref(),
            ) {
                (None, None) => {
                    legacy_preparations += 1;
                    anyhow::ensure!(
                        definition.benchmark_bundle.is_none(),
                        "legacy preparation {} points to workflow definition {} with a benchmark bundle",
                        preparation.id,
                        definition.id
                    );
                }
                (Some(bundle_id), Some(bundle_fingerprint)) => {
                    let bundle = bundles.get(&bundle_id).with_context(|| {
                        format!(
                            "preparation {} benchmark bundle is missing: {bundle_id}",
                            preparation.id
                        )
                    })?;
                    anyhow::ensure!(
                        bundle.fingerprint == bundle_fingerprint
                            && preparation.development_suite_id == bundle.development_suite_id
                            && preparation.sealed_suite_id == bundle.sealed_suite_id,
                        "preparation {} benchmark bundle projection differs",
                        preparation.id
                    );
                    definition
                        .benchmark_bundle
                        .as_ref()
                        .with_context(|| {
                            format!(
                                "preparation {} points to legacy workflow definition {}",
                                preparation.id, definition.id
                            )
                        })?
                        .validate_bundle(bundle)
                        .with_context(|| {
                            format!(
                                "preparation {} workflow benchmark bundle binding differs",
                                preparation.id
                            )
                        })?;
                }
                _ => anyhow::bail!(
                    "preparation {} has an incomplete benchmark bundle projection",
                    preparation.id
                ),
            }
        }

        Ok((
            bundles.len(),
            definitions.len().saturating_sub(legacy_definitions),
            preparations.len().saturating_sub(legacy_preparations),
            legacy_definitions + legacy_preparations,
        ))
    }
    .await;

    match result {
        Ok((bundles, definitions, preparations, legacy)) => pass(
            "benchmark_bundle_facts",
            format!(
                "{bundles} bundle(s), {definitions} workflow binding(s), and {preparations} preparation binding(s) verified; {legacy} legacy NULL binding(s) retained for inspection and marked non-executable"
            ),
        ),
        Err(error) => fail("benchmark_bundle_facts", error.to_string()),
    }
}

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
