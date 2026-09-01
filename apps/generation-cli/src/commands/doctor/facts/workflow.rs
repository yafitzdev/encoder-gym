use super::super::*;
use generation_supervisor_core::ports::GenerationSupervisorStore;

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
    use workflow_core::benchmark_qualification::{
        BenchmarkQualificationReviewDecision, BenchmarkReadiness,
    };
    use workflow_core::ports::BenchmarkQualificationQuery;
    use workflow_core::ports::BenchmarkQualificationStore;

    let result: anyhow::Result<(usize, usize, usize, usize)> = async {
        const PAGE_SIZE: u32 = 1_000;
        let mut bundle_page = Vec::new();
        let mut offset = 0_u32;
        loop {
            let page = store
                .query_benchmark_bundles(BenchmarkBundleQuery {
                development_suite_id: None,
                sealed_suite_id: None,
                contamination_report_id: None,
                    limit: PAGE_SIZE,
                    offset,
                })
                .await?;
            let page_len = page.len();
            bundle_page.extend(page);
            if page_len < PAGE_SIZE as usize {
                break;
            }
            offset = offset.saturating_add(PAGE_SIZE);
        }
        let bundles = bundle_page
            .iter()
            .map(|bundle| (bundle.id, bundle))
            .collect::<std::collections::BTreeMap<_, _>>();

        let mut qualification_page = Vec::new();
        offset = 0;
        loop {
            let page = store
                .query_benchmark_qualifications(BenchmarkQualificationQuery {
                    limit: PAGE_SIZE,
                    offset,
                    ..BenchmarkQualificationQuery::default()
                })
                .await?;
            let page_len = page.len();
            qualification_page.extend(page);
            if page_len < PAGE_SIZE as usize {
                break;
            }
            offset = offset.saturating_add(PAGE_SIZE);
        }
        let qualifications = qualification_page
            .iter()
            .map(|qualification| (qualification.id, qualification))
            .collect::<std::collections::BTreeMap<_, _>>();

        let mut definition_page = Vec::new();
        offset = 0;
        loop {
            let page = store
                .query_workflow_definitions(WorkflowDefinitionQuery {
                    dataset_id: None,
                    limit: PAGE_SIZE,
                    offset,
                })
                .await?;
            let page_len = page.len();
            definition_page.extend(page);
            if page_len < PAGE_SIZE as usize {
                break;
            }
            offset = offset.saturating_add(PAGE_SIZE);
        }
        let definitions = definition_page
            .iter()
            .map(|definition| (definition.id, definition))
            .collect::<std::collections::BTreeMap<_, _>>();

        let mut preparations = Vec::new();
        offset = 0;
        loop {
            let page = store.list_preparations(PAGE_SIZE, offset).await?;
            let page_len = page.len();
            preparations.extend(page);
            if page_len < PAGE_SIZE as usize {
                break;
            }
            offset = offset.saturating_add(PAGE_SIZE);
        }
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
            if let Some(qualification_binding) = &definition.benchmark_qualification {
                let qualification = qualifications
                    .get(&qualification_binding.qualification_id)
                    .with_context(|| {
                        format!(
                            "workflow definition {} benchmark qualification is missing: {}",
                            definition.id, qualification_binding.qualification_id
                        )
                    })?;
                anyhow::ensure!(
                    qualification.readiness == BenchmarkReadiness::Ready
                        && qualification.benchmark_bundle_id == binding.bundle_id
                        && qualification.benchmark_bundle_fingerprint == binding.bundle_fingerprint,
                    "workflow definition {} qualification is not ready for its bundle",
                    definition.id
                );
                let review = store
                    .get_benchmark_qualification_review(qualification_binding.review_id)
                    .await?
                    .with_context(|| {
                        format!(
                            "workflow definition {} benchmark qualification review is missing: {}",
                            definition.id, qualification_binding.review_id
                        )
                    })?;
                anyhow::ensure!(
                    review.decision == BenchmarkQualificationReviewDecision::Approve
                        && review.qualification_id == qualification.id
                        && review.fingerprint == qualification_binding.review_fingerprint,
                    "workflow definition {} benchmark qualification review is not approved and bound",
                    definition.id
                );
            }
        }

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
                    match (
                        preparation.benchmark_qualification_id,
                        preparation.benchmark_qualification_review_id,
                        preparation.benchmark_qualification_fingerprint.as_deref(),
                        preparation
                            .benchmark_qualification_review_fingerprint
                            .as_deref(),
                    ) {
                        (Some(qualification_id), Some(review_id), Some(qualification_fingerprint), Some(review_fingerprint)) => {
                            let definition_binding = definition
                                .benchmark_qualification
                                .as_ref()
                                .with_context(|| {
                                    format!(
                                        "preparation {} qualification projection points to an unqualified workflow",
                                        preparation.id
                                    )
                                })?;
                            anyhow::ensure!(
                                definition_binding.qualification_id == qualification_id
                                    && definition_binding.review_id == review_id
                                    && definition_binding.qualification_fingerprint == qualification_fingerprint
                                    && definition_binding.review_fingerprint == review_fingerprint,
                                "preparation {} qualification projection differs from workflow authority",
                                preparation.id
                            );
                        }
                        (None, None, None, None) => {}
                        _ => anyhow::bail!(
                            "preparation {} has an incomplete benchmark qualification projection",
                            preparation.id
                        ),
                    }
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
    use dataset_quality_core::ports::DatasetQualityStore;
    use workflow_core::execution::WorkflowChildKind;

    const PAGE_SIZE: u32 = 1_000;
    let mut definitions = Vec::new();
    let mut offset = 0_u32;
    loop {
        let page = match store
            .query_workflow_definitions(WorkflowDefinitionQuery {
                dataset_id: None,
                limit: PAGE_SIZE,
                offset,
            })
            .await
        {
            Ok(values) => values,
            Err(error) => return fail("workflow_facts", error.to_string()),
        };
        let page_len = page.len();
        definitions.extend(page);
        if page_len < PAGE_SIZE as usize {
            break;
        }
        offset = offset.saturating_add(PAGE_SIZE);
    }
    let mut runs = Vec::new();
    offset = 0;
    loop {
        let page = match store
            .query_workflow_runs(WorkflowRunQuery {
                definition_id: None,
                state: None,
                limit: PAGE_SIZE,
                offset,
            })
            .await
        {
            Ok(values) => values,
            Err(error) => return fail("workflow_facts", error.to_string()),
        };
        let page_len = page.len();
        runs.extend(page);
        if page_len < PAGE_SIZE as usize {
            break;
        }
        offset = offset.saturating_add(PAGE_SIZE);
    }
    let mut failures = Vec::new();
    let mut checked_children = 0_usize;
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
            match store.list_workflow_child_executions(attempt.id).await {
                Ok(children) => {
                    for (child_index, child) in children.iter().enumerate() {
                        if usize::try_from(child.ordinal).ok() != Some(child_index + 1) {
                            failures.push(format!(
                                "run {} attempt {} child execution ordinals are invalid",
                                run.id, attempt.id
                            ));
                            break;
                        }
                        let exists: anyhow::Result<bool> = match child.child_kind {
                            WorkflowChildKind::GenerationJob => store
                                .get_job(child.child_execution_id)
                                .await
                                .map(|value| value.is_some())
                                .map_err(Into::into),
                            WorkflowChildKind::GenerationSupervisorRun => store
                                .get_supervisor_run(child.child_execution_id)
                                .await
                                .map(|value| value.is_some())
                                .map_err(Into::into),
                            WorkflowChildKind::QualityAuditRun => store
                                .get_audit_run(child.child_execution_id)
                                .await
                                .map(|value| value.is_some())
                                .map_err(Into::into),
                            WorkflowChildKind::TrainingRun => store
                                .get_training_run(child.child_execution_id)
                                .await
                                .map(|value| value.is_some())
                                .map_err(Into::into),
                            WorkflowChildKind::EvaluationRun => store
                                .get_evaluation_run(child.child_execution_id)
                                .await
                                .map(|value| value.is_some())
                                .map_err(Into::into),
                        };
                        match exists {
                            Ok(true) => checked_children = checked_children.saturating_add(1),
                            Ok(false)
                                if run.latest_attempt_id == Some(attempt.id)
                                    || run.cancel_requested =>
                            {
                                checked_children = checked_children.saturating_add(1);
                            }
                            Ok(false) => failures.push(format!(
                                "run {} attempt {} child execution is missing: {}",
                                run.id, attempt.id, child.child_execution_id
                            )),
                            Err(error) => failures.push(format!(
                                "run {} attempt {} child execution failed validation: {error}",
                                run.id, attempt.id
                            )),
                        }
                    }
                }
                Err(error) => failures.push(format!(
                    "run {} attempt {} child execution links: {error}",
                    run.id, attempt.id
                )),
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
                "{} definition(s), {} run chain(s), {} child execution link(s), and {} workflow artifact(s) verified",
                definitions.len(),
                runs.len(),
                checked_children,
                checked_artifacts,
            ),
        )
    } else {
        fail("workflow_facts", failures.join("; "))
    }
}

pub(in crate::commands::doctor) async fn training_benchmark_facts_check(
    store: &SqliteStore,
) -> DoctorCheck {
    use workflow_core::{
        contamination::ContaminationStatus,
        ports::{
            ContaminationStore, PromotionStore, TrainingBenchmarkCheckQuery,
            TrainingBenchmarkCheckStore,
        },
        workflow::{StageAttemptState, WorkflowStage},
    };

    let result: anyhow::Result<(usize, usize, usize, usize, usize, usize)> = async {
        const PAGE_SIZE: u32 = 1_000;
        let mut all_checks = Vec::new();
        let mut offset = 0_u32;
        loop {
            let page = store
                .query_training_benchmark_checks(TrainingBenchmarkCheckQuery {
                training_snapshot_id: None,
                benchmark_bundle_id: None,
                status: None,
                protocol: None,
                check_protocol_version: None,
                limit: PAGE_SIZE,
                offset,
            })
                .await?;
            let returned = u32::try_from(page.len()).context("check page is too large")?;
            all_checks.extend(page);
            if returned < PAGE_SIZE {
                break;
            }
            offset = offset
                .checked_add(returned)
                .context("check audit offset overflow")?;
        }
        let checks = all_checks
            .into_iter()
            .map(|check| (check.id, check))
            .collect::<std::collections::BTreeMap<_, _>>();
        for check in checks.values() {
            check.validate_integrity()?;
            anyhow::ensure!(
                store
                    .get_contamination_override(check.contamination_report_id)
                    .await?
                    .is_none(),
                "training-benchmark check {} report has a forbidden override",
                check.id
            );
        }

        let migration_installed_at = sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
            "SELECT installed_on FROM _sqlx_migrations WHERE version = 47 AND success = 1",
        )
        .fetch_optional(store.pool())
        .await?;
        let promotion_migration_installed_at =
            sqlx::query_scalar::<_, chrono::DateTime<chrono::Utc>>(
                "SELECT installed_on FROM _sqlx_migrations WHERE version = 48 AND success = 1",
            )
            .fetch_optional(store.pool())
            .await?;
        let mut all_definitions = Vec::new();
        let mut offset = 0_u32;
        loop {
            let page = store
                .query_workflow_definitions(WorkflowDefinitionQuery {
                    dataset_id: None,
                    limit: PAGE_SIZE,
                    offset,
                })
                .await?;
            let returned = u32::try_from(page.len()).context("definition page is too large")?;
            all_definitions.extend(page);
            if returned < PAGE_SIZE {
                break;
            }
            offset = offset
                .checked_add(returned)
                .context("definition audit offset overflow")?;
        }
        let definitions = all_definitions
            .into_iter()
            .map(|definition| (definition.id, definition))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut runs = Vec::new();
        let mut offset = 0_u32;
        loop {
            let page = store
                .query_workflow_runs(WorkflowRunQuery {
                    definition_id: None,
                    state: None,
                    limit: PAGE_SIZE,
                    offset,
                })
                .await?;
            let returned = u32::try_from(page.len()).context("workflow run page is too large")?;
            runs.extend(page);
            if returned < PAGE_SIZE {
                break;
            }
            offset = offset
                .checked_add(returned)
                .context("workflow run audit offset overflow")?;
        }
        let mut verified_attempts = 0_usize;
        let mut invalid_attempts = 0_usize;
        let mut legacy_attempts = 0_usize;
        let mut verified_promotions = 0_usize;
        let mut legacy_promotions = 0_usize;
        for run in runs {
            let definition = definitions
                .get(&run.definition_id)
                .with_context(|| format!("workflow run {} definition is missing", run.id))?;
            let attempts = store.list_workflow_attempts(run.id).await?;
            for (index, attempt) in attempts.iter().enumerate().filter(|(_, attempt)| {
                matches!(
                    attempt.stage,
                    WorkflowStage::Training | WorkflowStage::IterationTraining
                ) && attempt.state != StageAttemptState::Running
            }) {
                let expected_kind = if attempt.stage == WorkflowStage::Training {
                    "training_benchmark_check"
                } else {
                    "iteration_training_benchmark_check"
                };
                let links = attempt
                    .artifacts
                    .iter()
                    .filter(|artifact| artifact.kind == expected_kind)
                    .collect::<Vec<_>>();
                if links.is_empty() {
                    let is_post_migration = migration_installed_at
                        .is_some_and(|installed| attempt.started_at >= installed);
                    if is_post_migration {
                        anyhow::ensure!(
                            !attempt.artifacts.iter().any(|artifact| matches!(
                                artifact.kind.as_str(),
                                "training_run"
                                    | "iteration_training_run"
                                    | "checkpoint"
                                    | "iteration_checkpoint"
                            )),
                            "governed training attempt {} retained trainer artifacts without a training-benchmark check",
                            attempt.id
                        );
                    }
                    let deterministic_invalid = attempt.state == StageAttemptState::Failed
                        && attempt.reason.as_deref().is_some_and(|reason| {
                            reason.starts_with("training-benchmark input invalid:")
                        });
                    if is_post_migration && deterministic_invalid {
                        anyhow::ensure!(
                            !attempt.retryable,
                            "invalid training-benchmark attempt {} is retryable",
                            attempt.id
                        );
                        invalid_attempts += 1;
                        continue;
                    }
                    if attempt.state == StageAttemptState::Completed && is_post_migration {
                        anyhow::bail!(
                            "completed governed training attempt {} has no training-benchmark check",
                            attempt.id
                        );
                    }
                    legacy_attempts += 1;
                    continue;
                }
                anyhow::ensure!(
                    links.len() == 1,
                    "training attempt {} has multiple training-benchmark checks",
                    attempt.id
                );
                let link = links[0];
                let check = checks.get(&link.artifact_id).with_context(|| {
                    format!(
                        "training attempt {} check is missing: {}",
                        attempt.id, link.artifact_id
                    )
                })?;
                anyhow::ensure!(
                    check.fingerprint == link.artifact_fingerprint,
                    "training attempt {} check fingerprint differs",
                    attempt.id
                );
                let bundle = definition
                    .benchmark_bundle
                    .as_ref()
                    .context("governed training definition has no benchmark bundle")?;
                let snapshot_kind = if attempt.stage == WorkflowStage::Training {
                    "snapshot"
                } else {
                    "iteration_snapshot"
                };
                let snapshot_id = attempts[..=index]
                    .iter()
                    .rev()
                    .flat_map(|value| value.artifacts.iter().rev())
                    .find(|artifact| artifact.kind == snapshot_kind)
                    .map(|artifact| artifact.artifact_id)
                    .with_context(|| {
                        format!("training attempt {} snapshot link is missing", attempt.id)
                    })?;
                anyhow::ensure!(
                    check.training_snapshot_id == snapshot_id
                        && check.benchmark_bundle_id == bundle.bundle_id
                        && check.benchmark_bundle_fingerprint == bundle.bundle_fingerprint,
                    "training attempt {} check authority differs",
                    attempt.id
                );
                match attempt.state {
                    StageAttemptState::Completed => {
                        anyhow::ensure!(
                            check.status == ContaminationStatus::Clean,
                            "completed training attempt {} used a blocked check",
                            attempt.id
                        );
                        let run_kind = if attempt.stage == WorkflowStage::Training {
                            "training_run"
                        } else {
                            "iteration_training_run"
                        };
                        let run_link = attempt
                            .artifacts
                            .iter()
                            .find(|artifact| artifact.kind == run_kind)
                            .with_context(|| {
                                format!("training attempt {} run link is missing", attempt.id)
                            })?;
                        let training_run = store
                            .get_training_run(run_link.artifact_id)
                            .await?
                            .with_context(|| {
                                format!(
                                    "training attempt {} run is missing: {}",
                                    attempt.id, run_link.artifact_id
                                )
                            })?;
                        let binding = training_run.input_binding.as_ref().with_context(|| {
                            format!(
                                "training attempt {} run has no immutable input binding",
                                attempt.id
                            )
                        })?;
                        store
                            .verify_training_run_input_authority(training_run.id, false)
                            .await?;
                        anyhow::ensure!(
                            artifact_core::fingerprint(&training_run)?
                                == run_link.artifact_fingerprint
                                && training_run.snapshot_id == check.training_snapshot_id
                                && binding.protocol
                                    == check.training_input_protocol.stable_name()
                                && binding.population_fingerprint
                                    == check.training_population_fingerprint
                                && binding.member_count == check.training_member_count
                                && binding.authority_kind == "training_benchmark_check"
                                && binding.authority_id == check.id
                                && binding.authority_fingerprint == check.fingerprint,
                            "training attempt {} run input authority differs from its check",
                            attempt.id
                        );
                    }
                    StageAttemptState::Failed if check.status == ContaminationStatus::Blocked => {
                        anyhow::ensure!(
                            !attempt.retryable
                                && !attempt.artifacts.iter().any(|artifact| matches!(
                                    artifact.kind.as_str(),
                                    "training_run"
                                        | "iteration_training_run"
                                        | "checkpoint"
                                        | "iteration_checkpoint"
                                )),
                            "blocked training attempt {} retained trainer artifacts or is retryable",
                            attempt.id
                        );
                    }
                    _ => {}
                }
                verified_attempts += 1;
            }

            if let Some(promotion) = store.get_workflow_promotion(run.id).await? {
                let (Some(check_id), Some(check_fingerprint)) = (
                    promotion.training_benchmark_check_id,
                    promotion.training_benchmark_check_fingerprint.as_deref(),
                ) else {
                    let is_post_migration = promotion_migration_installed_at
                        .is_some_and(|installed| promotion.created_at >= installed);
                    anyhow::ensure!(
                        !is_post_migration,
                        "post-migration promotion {} has no training-benchmark check",
                        promotion.id
                    );
                    legacy_promotions += 1;
                    continue;
                };
                let check = checks.get(&check_id).with_context(|| {
                    format!(
                        "promotion {} training-benchmark check is missing: {check_id}",
                        promotion.id
                    )
                })?;
                let bundle = definition
                    .benchmark_bundle
                    .as_ref()
                    .context("promotion workflow definition has no benchmark bundle")?;
                anyhow::ensure!(
                    check.fingerprint == check_fingerprint
                        && check.status == ContaminationStatus::Clean
                        && check.training_snapshot_id == promotion.training_snapshot_id
                        && check.training_snapshot_fingerprint
                            == promotion.training_snapshot_fingerprint
                        && check.benchmark_bundle_id == bundle.bundle_id
                        && check.benchmark_bundle_fingerprint == bundle.bundle_fingerprint,
                    "promotion {} training-benchmark authority differs",
                    promotion.id
                );
                let expected_kind = if attempts.iter().any(|attempt| {
                    attempt
                        .artifacts
                        .iter()
                        .any(|artifact| artifact.kind == "iteration_checkpoint")
                }) {
                    "iteration_training_benchmark_check"
                } else {
                    "training_benchmark_check"
                };
                let expected_link = attempts
                    .iter()
                    .rev()
                    .flat_map(|attempt| attempt.artifacts.iter().rev())
                    .find(|artifact| artifact.kind == expected_kind)
                    .with_context(|| {
                        format!(
                            "promotion {} has no final-cycle training-benchmark link",
                            promotion.id
                        )
                    })?;
                anyhow::ensure!(
                    expected_link.artifact_id == check_id
                        && expected_link.artifact_fingerprint == check_fingerprint,
                    "promotion {} does not pin its final training cycle's check",
                    promotion.id
                );
                verified_promotions += 1;
            }
        }
        Ok((
            checks.len(),
            verified_attempts,
            legacy_attempts,
            invalid_attempts,
            verified_promotions,
            legacy_promotions,
        ))
    }
    .await;

    match result {
        Ok((
            checks,
            attempts,
            legacy_attempts,
            invalid_attempts,
            promotions,
            legacy_promotions,
        )) => pass(
            "training_benchmark_facts",
            format!(
                "{checks} immutable check(s), {attempts} governed training attempt(s), {invalid_attempts} deterministic invalid-input stop(s), and {promotions} promotion binding(s) verified; {legacy_attempts} legacy or pre-clearance attempt(s) and {legacy_promotions} legacy promotion(s) remain explicitly unverified"
            ),
        ),
        Err(error) => fail("training_benchmark_facts", error.to_string()),
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
