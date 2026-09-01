use super::{quality_gate, training_benchmark_gate, *};
use crate::commands::{evaluation, generation, optimization, snapshot, training};
use workflow_core::execution::WorkflowChildKind;

pub(super) fn execute_initial_stage<'a>(
    store: &'a SqliteStore,
    definition: &'a WorkflowDefinition,
    run: &'a WorkflowRun,
    attempt: &'a WorkflowStageAttempt,
    configured: &'a project_config::ResolvedProjectConfig,
    benchmark_authority: &'a WorkflowBenchmarkAuthority,
) -> Pin<Box<dyn Future<Output = anyhow::Result<StageExecution>> + Send + 'a>> {
    Box::pin(async move {
        let history = store.list_workflow_attempts(run.id).await?;
        let mut usage = run.usage.clone();
        if attempt.stage == WorkflowStage::Approval {
            return execute_approval_stage(store, definition, run, configured, usage).await;
        }
        let artifacts = match attempt.stage {
            WorkflowStage::InitialAllocation => {
                let record = execute_initial_allocation(store, definition).await?;
                vec![
                    link("initial_allocation", record.id, &record.fingerprint),
                    link(
                        "generation_plan",
                        record.generation_plan_id,
                        &record.result.fingerprint,
                    ),
                ]
            }
            WorkflowStage::Generation => {
                let plan_id = artifact_id(&history, "generation_plan")?;
                if definition.generation_supervision.is_some() {
                    return super::generation_supervision::execute(
                        store, definition, run, attempt, configured, &history, plan_id,
                    )
                    .await;
                }
                let allocation_id = artifact_id(&history, "initial_allocation")?;
                let allocation = store
                    .get_initial_allocation(allocation_id)
                    .await?
                    .context("workflow initial allocation not found")?;
                let existing = store
                    .list_jobs(JobQuery {
                        plan_id: Some(plan_id),
                        state: Some(JobState::Completed),
                        limit: 1,
                        ..JobQuery::default()
                    })
                    .await?
                    .into_iter()
                    .next();
                let job = match existing {
                    Some(job) => job,
                    None => {
                        let child = child_execution::reserve_next(
                            store,
                            attempt,
                            WorkflowChildKind::GenerationJob,
                            "primary",
                        )
                        .await?;
                        generation::run_workflow(plan_id, configured, &child, store.clone()).await?
                    }
                };
                ensure!(
                    job.state == JobState::Completed,
                    "generation job did not complete"
                );
                usage.accepted_rows = u64::from(allocation.result.initial_target_rows);
                usage.generation_attempts =
                    usage.generation_attempts.saturating_add(job.generated_rows);
                let batch = u64::from(configured.generation.batch_size.max(1));
                usage.generation_requests = usage
                    .generation_requests
                    .saturating_add(job.generated_rows.div_ceil(batch))
                    .saturating_add(job.failed_requests);
                vec![link(
                    "generation_job",
                    job.id,
                    &artifact_core::fingerprint(&job)?,
                )]
            }
            WorkflowStage::QualityAudit | WorkflowStage::IterationQualityAudit => {
                quality_gate::execute_audit(store, definition, run, attempt)
                    .await?
                    .links
            }
            WorkflowStage::CurationReview | WorkflowStage::IterationCurationReview => {
                let proposal =
                    quality_gate::create_review_proposal(store, &history, attempt.stage).await?;
                let proposal_kind = quality_gate::proposal_kind(attempt.stage)?;
                return Ok(StageExecution {
                    artifacts: vec![link(proposal_kind, proposal.id, &proposal.fingerprint)],
                    usage,
                    state: StageAttemptState::AwaitingUser,
                    reason: Some(format!(
                        "explicit approval of exact curation proposal {} is required; review it with `quality proposal {}` and approve it with `quality manifest-review {} --approve --reviewer <name> --reason <reason>`, then resume this workflow",
                        proposal.id, proposal.id, proposal.id
                    )),
                });
            }
            WorkflowStage::CurationApproval | WorkflowStage::IterationCurationApproval => {
                let review_stage = if attempt.stage == WorkflowStage::CurationApproval {
                    WorkflowStage::CurationReview
                } else {
                    WorkflowStage::IterationCurationReview
                };
                let evidence =
                    quality_gate::approved_manifest(store, definition, &history, review_stage)
                        .await?
                        .approved
                        .context("the exact latest workflow curation proposal is not approved")?;
                let proposal_kind = quality_gate::proposal_kind(review_stage)?;
                let manifest_kind = if attempt.stage == WorkflowStage::CurationApproval {
                    "quality_manifest"
                } else {
                    "iteration_quality_manifest"
                };
                vec![
                    link(
                        proposal_kind,
                        evidence.proposal.id,
                        &evidence.proposal.fingerprint,
                    ),
                    link(
                        "curation_manifest_review",
                        evidence.manifest.approval_id,
                        &evidence.manifest.approval_fingerprint,
                    ),
                    link(
                        manifest_kind,
                        evidence.manifest.id,
                        &evidence.manifest.fingerprint,
                    ),
                ]
            }
            WorkflowStage::Snapshot => {
                let manifest_id = (definition.quality_gate.is_some()
                    || definition.generation_supervision.is_some())
                .then_some(())
                .map(|_| artifact_id(&history, "quality_manifest"))
                .transpose()?;
                let result = snapshot::create_workflow(
                    definition.dataset_id,
                    run.id,
                    run.iteration,
                    configured,
                    manifest_id,
                    store,
                )
                .await?;
                let mut links = vec![link(
                    "snapshot",
                    result.snapshot.id,
                    &result.snapshot.fingerprint,
                )];
                if let Some(application) = result.curation_application {
                    links.push(link(
                        "curation_application",
                        application.id,
                        &application.fingerprint,
                    ));
                }
                links
            }
            WorkflowStage::Training => {
                let snapshot_id = artifact_id(&history, "snapshot")?;
                if definition.quality_gate.is_some() || definition.generation_supervision.is_some()
                {
                    let manifest_id = artifact_id(&history, "quality_manifest")?;
                    snapshot::require_workflow_qualification(store, snapshot_id, manifest_id)
                        .await?;
                }
                let check = match training_benchmark_gate::ensure_check(
                    store,
                    definition,
                    snapshot_id,
                    benchmark_authority,
                )
                .await?
                {
                    training_benchmark_gate::GateResult::Checked(check) => check,
                    training_benchmark_gate::GateResult::DeterministicallyInvalid(reason) => {
                        return Ok(StageExecution::failed(Vec::new(), usage, reason));
                    }
                };
                let check_link = link("training_benchmark_check", check.id, &check.fingerprint);
                if !check.training_allowed() {
                    return Ok(StageExecution::failed(
                        vec![check_link],
                        usage,
                        format!(
                            "training blocked before backend startup by immutable benchmark-leakage check {}; inspect it with `benchmark training-check-show {}`",
                            check.id, check.id
                        ),
                    ));
                }
                let input = training::prepare_workflow_input(snapshot_id, &check, store).await?;
                let binding = input.binding().clone();
                let candidates = training_core::ports::TrainingStore::query_training_runs(
                    store,
                    training_core::ports::TrainingRunQuery {
                        snapshot_id: Some(snapshot_id),
                        input_authority_id: Some(check.id),
                        state: Some(training_core::domain::TrainingRunState::Completed),
                        limit: 10_000,
                        offset: 0,
                    },
                )
                .await?;
                let mut eligible = Vec::new();
                for run in candidates {
                    if training::reusable_workflow_run(&run, snapshot_id, configured, &binding)? {
                        eligible.push(run);
                    }
                }
                ensure!(
                    eligible.len() <= 1,
                    "multiple completed training runs claim the same immutable workflow input"
                );
                let existing = eligible.pop();
                let completed = match existing {
                    Some(run) => training::CompletedTraining {
                        checkpoints: training_core::ports::TrainingStore::list_checkpoints(
                            store, run.id,
                        )
                        .await?,
                        run,
                    },
                    None => {
                        let child = child_execution::reserve_next(
                            store,
                            attempt,
                            WorkflowChildKind::TrainingRun,
                            "primary",
                        )
                        .await?;
                        training::run_workflow(configured, input, &child, store.clone()).await?
                    }
                };
                let checkpoint = completed
                    .checkpoints
                    .iter()
                    .find(|value| value.is_final)
                    .context("training completed without a final checkpoint")?;
                vec![
                    check_link,
                    link(
                        "training_run",
                        completed.run.id,
                        &artifact_core::fingerprint(&completed.run)?,
                    ),
                    link("checkpoint", checkpoint.id, &checkpoint.artifact_checksum),
                ]
            }
            WorkflowStage::DevelopmentEvaluation => {
                require_stage_training_check(
                    store,
                    &history,
                    "training_benchmark_check",
                    "snapshot",
                    benchmark_authority,
                )
                .await?;
                let checkpoint_id = artifact_id(&history, "checkpoint")?;
                let suite = &benchmark_authority.development;
                let mut links = Vec::new();
                for cohort in &suite.cohorts {
                    let existing = store
                        .query_evaluation_runs(evaluation_core::ports::EvaluationRunQuery {
                            checkpoint_id: Some(checkpoint_id),
                            snapshot_id: Some(cohort.snapshot_id),
                            state: Some(evaluation_core::domain::EvaluationRunState::Completed),
                            limit: 100,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|value| {
                            value.protocol_fingerprint == cohort.protocol_fingerprint
                                && value.split == cohort.split
                        });
                    let evaluation = match existing {
                        Some(evaluation) => evaluation,
                        None => {
                            let child = child_execution::reserve_next(
                                store,
                                attempt,
                                WorkflowChildKind::EvaluationRun,
                                cohort.cohort_id.to_string(),
                            )
                            .await?;
                            evaluation::run_workflow(
                                checkpoint_id,
                                cohort.snapshot_id,
                                &cohort.protocol,
                                &child,
                                store.clone(),
                            )
                            .await?
                            .run
                        }
                    };
                    links.push(link(
                        "evaluation_run",
                        evaluation.id,
                        &artifact_core::fingerprint(&evaluation)?,
                    ));
                }
                links
            }
            WorkflowStage::AcceptanceAssessment => {
                let suite = &benchmark_authority.development;
                validate_suite_access(suite, ExposurePurpose::DevelopmentEvaluation, None)?;
                let evaluation_ids = artifact_ids(&history, "evaluation_run");
                let mut inputs = Vec::new();
                for cohort in &suite.cohorts {
                    inputs.push(CohortAssessmentInput {
                        cohort_id: cohort.cohort_id,
                        run: Some(load_matching_evaluation(store, cohort, &evaluation_ids).await?),
                        comparison: None,
                    });
                }
                let assessment = assess_benchmark(suite, inputs)?;
                let existing = store
                    .query_acceptance_assessments(AcceptanceAssessmentQuery {
                        suite_id: Some(suite.id),
                        checkpoint_id: assessment.checkpoint_id,
                        state: None,
                        limit: 10_000,
                        offset: 0,
                    })
                    .await?
                    .into_iter()
                    .find(|value| value.evaluation_run_ids == assessment.evaluation_run_ids);
                let assessment = match existing {
                    Some(value) => value,
                    None => {
                        store.create_acceptance_assessment(&assessment).await?;
                        assessment
                    }
                };
                record_suite_exposures(
                    store,
                    suite,
                    assessment.evaluation_run_ids.values().copied().collect(),
                    run,
                    ExposurePurpose::DevelopmentEvaluation,
                    None,
                    "workflow development assessment",
                )
                .await?;
                let mut links = vec![link(
                    "acceptance_assessment",
                    assessment.id,
                    &assessment.fingerprint,
                )];
                if assessment.state == workflow_core::benchmark::AcceptanceState::Pass {
                    links.push(link(
                        "development_acceptance_pass",
                        assessment.id,
                        &assessment.fingerprint,
                    ));
                }
                links
            }
            WorkflowStage::ErrorAnalysis => {
                let evaluation_ids = artifact_ids(&history, "evaluation_run");
                let suite = &benchmark_authority.development;
                validate_suite_access(
                    suite,
                    ExposurePurpose::Diagnosis,
                    Some(DisclosureLevel::RowContent),
                )?;
                let protocol = definition
                    .analysis_protocol
                    .as_ref()
                    .context("legacy workflow definition has no resolved analysis protocol")?;
                let protocol_fingerprint = protocol.fingerprint()?;
                let mut links = Vec::new();
                for evaluation_id in evaluation_ids {
                    let existing = store
                        .query_analysis_reports(analysis_core::ports::AnalysisReportQuery {
                            evaluation_run_id: Some(evaluation_id),
                            limit: 10_000,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|report| report.protocol_fingerprint == protocol_fingerprint);
                    let report = match existing {
                        Some(report) => report,
                        None => run_analysis(store, store, evaluation_id, protocol.clone()).await?,
                    };
                    links.push(link("analysis_report", report.id, &report.fingerprint));
                }
                ensure!(
                    !links.is_empty(),
                    "development analysis produced no reports"
                );
                record_suite_exposures(
                    store,
                    suite,
                    artifact_ids(&history, "evaluation_run"),
                    run,
                    ExposurePurpose::Diagnosis,
                    Some(DisclosureLevel::RowContent),
                    "workflow error analysis",
                )
                .await?;
                links
            }
            WorkflowStage::Advisor => {
                let configuration = definition
                    .advisor
                    .as_ref()
                    .context("workflow advisor is enabled without resolved configuration")?;
                let suite = &benchmark_authority.development;
                let advisor_disclosure = match configuration.egress_policy {
                    AdvisorEgressPolicy::AggregateOnly => DisclosureLevel::Aggregate,
                    AdvisorEgressPolicy::DevelopmentText => DisclosureLevel::RowContent,
                };
                validate_suite_access(suite, ExposurePurpose::Advisor, Some(advisor_disclosure))?;
                let analysis_report_id = artifact_id(&history, "analysis_report")?;
                let assessment_id = artifact_id(&history, "acceptance_assessment")?;
                let report = store
                    .get_analysis_report(analysis_report_id)
                    .await?
                    .context("workflow analysis report not found")?;
                let acceptance = store
                    .get_acceptance_assessment(assessment_id)
                    .await?
                    .context("workflow acceptance assessment not found")?;
                let existing = store
                    .query_advisory_assessments(AdvisoryAssessmentQuery {
                        workflow_run_id: Some(run.id),
                        analysis_report_id: Some(report.id),
                        limit: 10_000,
                        offset: 0,
                    })
                    .await?
                    .into_iter()
                    .find(|value| {
                        value.request.workflow_iteration == run.iteration
                            && value.backend == configuration.backend
                            && value.model == configuration.model
                    });
                let assessment = match existing {
                    Some(value) => value,
                    None => {
                        let dataset = store
                            .get_dataset(definition.dataset_id)
                            .await?
                            .context("workflow dataset not found")?;
                        let finding_limit = usize::from(configuration.maximum_findings);
                        let findings = report
                            .findings
                            .iter()
                            .take(finding_limit)
                            .map(|finding| AdvisoryFinding {
                                key: finding.key.clone(),
                                fingerprint: finding.fingerprint.clone(),
                                kind: serde_json::to_value(finding.kind)
                                    .ok()
                                    .and_then(|value| value.as_str().map(str::to_owned))
                                    .unwrap_or_else(|| "unknown".into()),
                                support: finding.support,
                                error_count: finding.error_count,
                                error_rate: finding.error_rate,
                            })
                            .collect();
                        let representative_errors = if configuration.egress_policy
                            == AdvisorEgressPolicy::DevelopmentText
                        {
                            report
                                .errors
                                .iter()
                                .take(usize::from(configuration.maximum_representative_errors))
                                .map(|error| AdvisoryExample {
                                    finding_key: report
                                        .findings
                                        .first()
                                        .map_or_else(String::new, |finding| finding.key.clone()),
                                    text: error.text.clone(),
                                    expected_label: error.expected_label.clone(),
                                    predicted_label: error.predicted_label.clone(),
                                    dimensions: error.dimensions.clone(),
                                })
                                .collect()
                        } else {
                            Vec::new()
                        };
                        let request = AdvisoryRequest {
                            id: uuid::Uuid::new_v4(),
                            workflow_run_id: run.id,
                            workflow_iteration: run.iteration,
                            task: dataset.task_description.clone(),
                            labels: dataset.labels.clone(),
                            dimensions: dataset
                                .dimensions
                                .iter()
                                .map(|dimension| (dimension.name.clone(), dimension.values.clone()))
                                .collect(),
                            semantic_context_job_id: artifact_id(&history, "generation_job").ok(),
                            semantic_context: match artifact_id(&history, "generation_job").ok() {
                                Some(job_id) => store
                                    .get_generation_semantics(job_id)
                                    .await?
                                    .map(|assignment| assignment.context),
                                None => None,
                            },
                            analysis_report_id: report.id,
                            analysis_report_fingerprint: report.fingerprint.clone(),
                            acceptance_assessment_id: acceptance.id,
                            acceptance_assessment_fingerprint: acceptance.fingerprint.clone(),
                            acceptance_state: serde_json::to_value(acceptance.state)?
                                .as_str()
                                .unwrap_or("invalid")
                                .to_owned(),
                            prediction_count: report.prediction_count,
                            error_count: report.error_count,
                            findings,
                            representative_errors,
                            allowed_cells: expand_generation_cells(&dataset),
                            allowed_actions: vec![
                                AdvisoryActionKind::Stop,
                                AdvisoryActionKind::Inspect,
                                AdvisoryActionKind::ConsiderExperiment,
                            ],
                            remaining_row_budget: definition
                                .budget
                                .maximum_cumulative_rows
                                .saturating_sub(usage.accepted_rows),
                            remaining_iteration_budget: definition
                                .budget
                                .maximum_iterations
                                .saturating_sub(usage.iterations),
                            egress_policy: configuration.egress_policy,
                        };
                        let backend: Box<dyn AnalysisAdvisor> = match configuration.backend.as_str()
                        {
                            "fake" => Box::<FakeAnalysisAdvisor>::default(),
                            "openai-compatible" => {
                                let base_url = configuration
                                    .base_url
                                    .as_deref()
                                    .context("OpenAI-compatible advisor requires base_url")?;
                                let api_key = std::env::var(&configuration.api_key_env).ok();
                                Box::new(OpenAICompatibleAdvisor::new(base_url, api_key)?)
                            }
                            value => anyhow::bail!("unsupported advisor backend: {value}"),
                        };
                        let value = run_advisor(backend.as_ref(), configuration, request).await?;
                        store.create_advisory_assessment(&value).await?;
                        value
                    }
                };
                record_suite_exposures(
                    store,
                    suite,
                    artifact_ids(&history, "evaluation_run"),
                    run,
                    ExposurePurpose::Advisor,
                    Some(advisor_disclosure),
                    "workflow advisory assessment",
                )
                .await?;
                usage.advisor_calls = usage.advisor_calls.saturating_add(1);
                usage.advisor_tokens = usage.advisor_tokens.saturating_add(
                    assessment.usage.total_tokens.unwrap_or_else(|| {
                        assessment.usage.input_tokens.unwrap_or_default()
                            + assessment.usage.output_tokens.unwrap_or_default()
                    }),
                );
                vec![link(
                    "advisory_assessment",
                    assessment.id,
                    &assessment.fingerprint,
                )]
            }
            WorkflowStage::OptimizationProposal => {
                let suite = &benchmark_authority.development;
                validate_suite_access(
                    suite,
                    ExposurePurpose::Optimization,
                    Some(DisclosureLevel::Slices),
                )?;
                let analysis_report_id = artifact_id(&history, "followup_analysis_report")
                    .or_else(|_| artifact_id(&history, "analysis_report"))?;
                let report = store
                    .get_analysis_report(analysis_report_id)
                    .await?
                    .context("workflow analysis report not found")?;
                let proposal = optimization::propose_workflow(
                    store,
                    analysis_report_id,
                    definition.optimization_protocol.as_ref().context(
                        "legacy workflow definition has no resolved optimization protocol",
                    )?,
                )
                .await?;
                record_suite_exposures(
                    store,
                    suite,
                    vec![report.evaluation_run_id],
                    run,
                    ExposurePurpose::Optimization,
                    Some(DisclosureLevel::Slices),
                    "workflow optimization proposal",
                )
                .await?;
                vec![link(
                    "optimization_proposal",
                    proposal.id,
                    &proposal.fingerprint,
                )]
            }
            WorkflowStage::ProposalApplication => {
                let decision = store
                    .get_iteration_workflow_approval(run.id, run.iteration)
                    .await?
                    .context("workflow proposal has no approval decision")?;
                let (plan, application) = optimization::apply_workflow(
                    store,
                    decision.proposal_id,
                    decision.proposal_review_id,
                    definition.dataset_id,
                )
                .await?;
                vec![
                    link("workflow_approval", decision.id, &decision.fingerprint),
                    link(
                        "proposal_review",
                        decision.proposal_review_id,
                        &decision.proposal_review_fingerprint,
                    ),
                    link(
                        "proposal_application",
                        application.proposal_id,
                        &artifact_core::fingerprint(&application)?,
                    ),
                    link(
                        "iteration_generation_plan",
                        plan.id,
                        &artifact_core::fingerprint(&plan)?,
                    ),
                ]
            }
            WorkflowStage::DatasetDiffGeneration => {
                let plan_id = artifact_id(&history, "iteration_generation_plan")?;
                if definition.generation_supervision.is_some() {
                    return super::generation_supervision::execute(
                        store, definition, run, attempt, configured, &history, plan_id,
                    )
                    .await;
                }
                let existing = store
                    .list_jobs(JobQuery {
                        plan_id: Some(plan_id),
                        state: Some(JobState::Completed),
                        limit: 1,
                        ..JobQuery::default()
                    })
                    .await?
                    .into_iter()
                    .next();
                let job = match existing {
                    Some(job) => job,
                    None => {
                        let child = child_execution::reserve_next(
                            store,
                            attempt,
                            WorkflowChildKind::GenerationJob,
                            "primary",
                        )
                        .await?;
                        generation::run_workflow(plan_id, configured, &child, store.clone()).await?
                    }
                };
                ensure!(
                    job.state == JobState::Completed,
                    "dataset diff did not complete"
                );
                usage.accepted_rows = usage.accepted_rows.saturating_add(job.accepted_rows);
                usage.generation_attempts =
                    usage.generation_attempts.saturating_add(job.generated_rows);
                let batch = u64::from(configured.generation.batch_size.max(1));
                usage.generation_requests = usage
                    .generation_requests
                    .saturating_add(job.generated_rows.div_ceil(batch))
                    .saturating_add(job.failed_requests);
                usage.iterations = usage.iterations.saturating_add(1);
                vec![link(
                    "dataset_diff_generation_job",
                    job.id,
                    &artifact_core::fingerprint(&job)?,
                )]
            }
            WorkflowStage::IterationSnapshot => {
                let manifest_id = (definition.quality_gate.is_some()
                    || definition.generation_supervision.is_some())
                .then_some(())
                .map(|_| artifact_id(&history, "iteration_quality_manifest"))
                .transpose()?;
                let result = snapshot::create_workflow(
                    definition.dataset_id,
                    run.id,
                    run.iteration,
                    configured,
                    manifest_id,
                    store,
                )
                .await?;
                let mut links = vec![link(
                    "iteration_snapshot",
                    result.snapshot.id,
                    &result.snapshot.fingerprint,
                )];
                if let Some(application) = result.curation_application {
                    links.push(link(
                        "iteration_curation_application",
                        application.id,
                        &application.fingerprint,
                    ));
                }
                links
            }
            WorkflowStage::IterationTraining => {
                ensure!(
                    definition
                        .training_iteration_policy
                        .unwrap_or(TrainingIterationPolicy::Fresh)
                        == TrainingIterationPolicy::Fresh,
                    "unsupported workflow iteration training policy"
                );
                let snapshot_id = artifact_id(&history, "iteration_snapshot")?;
                if definition.quality_gate.is_some() || definition.generation_supervision.is_some()
                {
                    let manifest_id = artifact_id(&history, "iteration_quality_manifest")?;
                    snapshot::require_workflow_qualification(store, snapshot_id, manifest_id)
                        .await?;
                }
                let check = match training_benchmark_gate::ensure_check(
                    store,
                    definition,
                    snapshot_id,
                    benchmark_authority,
                )
                .await?
                {
                    training_benchmark_gate::GateResult::Checked(check) => check,
                    training_benchmark_gate::GateResult::DeterministicallyInvalid(reason) => {
                        return Ok(StageExecution::failed(Vec::new(), usage, reason));
                    }
                };
                let check_link = link(
                    "iteration_training_benchmark_check",
                    check.id,
                    &check.fingerprint,
                );
                if !check.training_allowed() {
                    return Ok(StageExecution::failed(
                        vec![check_link],
                        usage,
                        format!(
                            "iteration training blocked before backend startup by immutable benchmark-leakage check {}; inspect it with `benchmark training-check-show {}`",
                            check.id, check.id
                        ),
                    ));
                }
                let input = training::prepare_workflow_input(snapshot_id, &check, store).await?;
                let binding = input.binding().clone();
                let candidates = training_core::ports::TrainingStore::query_training_runs(
                    store,
                    training_core::ports::TrainingRunQuery {
                        snapshot_id: Some(snapshot_id),
                        input_authority_id: Some(check.id),
                        state: Some(training_core::domain::TrainingRunState::Completed),
                        limit: 10_000,
                        offset: 0,
                    },
                )
                .await?;
                let mut eligible = Vec::new();
                for run in candidates {
                    if training::reusable_workflow_run(&run, snapshot_id, configured, &binding)? {
                        eligible.push(run);
                    }
                }
                ensure!(
                    eligible.len() <= 1,
                    "multiple completed iteration training runs claim the same immutable workflow input"
                );
                let existing = eligible.pop();
                let completed = match existing {
                    Some(training_run) => training::CompletedTraining {
                        checkpoints: training_core::ports::TrainingStore::list_checkpoints(
                            store,
                            training_run.id,
                        )
                        .await?,
                        run: training_run,
                    },
                    None => {
                        let child = child_execution::reserve_next(
                            store,
                            attempt,
                            WorkflowChildKind::TrainingRun,
                            "primary",
                        )
                        .await?;
                        training::run_workflow(configured, input, &child, store.clone()).await?
                    }
                };
                let checkpoint = completed
                    .checkpoints
                    .iter()
                    .find(|value| value.is_final)
                    .context("iteration training has no final checkpoint")?;
                vec![
                    check_link,
                    link(
                        "iteration_training_run",
                        completed.run.id,
                        &artifact_core::fingerprint(&completed.run)?,
                    ),
                    link(
                        "iteration_checkpoint",
                        checkpoint.id,
                        &checkpoint.artifact_checksum,
                    ),
                ]
            }
            WorkflowStage::IterationEvaluation => {
                require_stage_training_check(
                    store,
                    &history,
                    "iteration_training_benchmark_check",
                    "iteration_snapshot",
                    benchmark_authority,
                )
                .await?;
                let checkpoint_id = artifact_id(&history, "iteration_checkpoint")?;
                let suite = &benchmark_authority.development;
                let mut links = Vec::new();
                for cohort in &suite.cohorts {
                    let evaluation = match store
                        .query_evaluation_runs(evaluation_core::ports::EvaluationRunQuery {
                            checkpoint_id: Some(checkpoint_id),
                            snapshot_id: Some(cohort.snapshot_id),
                            state: Some(evaluation_core::domain::EvaluationRunState::Completed),
                            limit: 100,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|value| {
                            value.protocol_fingerprint == cohort.protocol_fingerprint
                                && value.split == cohort.split
                        }) {
                        Some(value) => value,
                        None => {
                            let child = child_execution::reserve_next(
                                store,
                                attempt,
                                WorkflowChildKind::EvaluationRun,
                                cohort.cohort_id.to_string(),
                            )
                            .await?;
                            evaluation::run_workflow(
                                checkpoint_id,
                                cohort.snapshot_id,
                                &cohort.protocol,
                                &child,
                                store.clone(),
                            )
                            .await?
                            .run
                        }
                    };
                    links.push(link(
                        "iteration_evaluation_run",
                        evaluation.id,
                        &artifact_core::fingerprint(&evaluation)?,
                    ));
                }
                links
            }
            WorkflowStage::Comparison => {
                let suite = &benchmark_authority.development;
                validate_suite_access(
                    suite,
                    ExposurePurpose::Comparison,
                    Some(DisclosureLevel::Predictions),
                )?;
                let baseline_ids = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::DevelopmentEvaluation,
                    0,
                    "evaluation_run",
                );
                let iteration_ids = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::IterationEvaluation,
                    run.iteration,
                    "iteration_evaluation_run",
                );
                ensure!(
                    baseline_ids.len() == suite.cohorts.len()
                        && iteration_ids.len() == suite.cohorts.len()
                        && !suite.cohorts.is_empty(),
                    "iteration evaluations are incompatible with the benchmark suite"
                );
                let mut links = Vec::new();
                for cohort in &suite.cohorts {
                    let baseline = load_matching_evaluation(store, cohort, &baseline_ids).await?;
                    let iteration = load_matching_evaluation(store, cohort, &iteration_ids).await?;
                    let comparison =
                        evaluation::compare_workflow(baseline.id, iteration.id, store).await?;
                    links.push(link(
                        "evaluation_comparison",
                        comparison.id,
                        &comparison.fingerprint,
                    ));
                }
                record_suite_exposures(
                    store,
                    suite,
                    iteration_ids,
                    run,
                    ExposurePurpose::Comparison,
                    Some(DisclosureLevel::Predictions),
                    "workflow paired evaluation comparison",
                )
                .await?;
                links
            }
            WorkflowStage::FollowupAnalysis => {
                let suite = &benchmark_authority.development;
                validate_suite_access(
                    suite,
                    ExposurePurpose::Diagnosis,
                    Some(DisclosureLevel::RowContent),
                )?;
                let evaluations = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::IterationEvaluation,
                    run.iteration,
                    "iteration_evaluation_run",
                );
                let comparisons = artifact_ids_for_stage(
                    &history,
                    WorkflowStage::Comparison,
                    run.iteration,
                    "evaluation_comparison",
                );
                ensure!(
                    evaluations.len() == comparisons.len() && !evaluations.is_empty(),
                    "follow-up analysis inputs are incomplete"
                );
                let comparisons_by_candidate =
                    load_comparisons_by_candidate(store, &comparisons).await?;
                ensure!(
                    comparisons_by_candidate.len() == evaluations.len(),
                    "follow-up analysis comparisons do not map one-to-one to candidate evaluations"
                );
                let exposure_evaluations = evaluations.clone();
                let base_protocol = definition
                    .analysis_protocol
                    .as_ref()
                    .context("workflow has no resolved analysis protocol")?;
                let mut links = Vec::new();
                for evaluation_id in evaluations {
                    let comparison_id = comparisons_by_candidate
                        .get(&evaluation_id)
                        .context(
                            "follow-up analysis has no cohort-compatible comparison for its candidate evaluation",
                        )?
                        .id;
                    let mut protocol = base_protocol.clone();
                    protocol.comparison_id = Some(comparison_id);
                    let fingerprint = protocol.fingerprint()?;
                    let existing = store
                        .query_analysis_reports(analysis_core::ports::AnalysisReportQuery {
                            evaluation_run_id: Some(evaluation_id),
                            limit: 10_000,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|report| report.protocol_fingerprint == fingerprint);
                    let report = match existing {
                        Some(value) => value,
                        None => run_analysis(store, store, evaluation_id, protocol).await?,
                    };
                    links.push(link(
                        "followup_analysis_report",
                        report.id,
                        &report.fingerprint,
                    ));
                }
                record_suite_exposures(
                    store,
                    suite,
                    exposure_evaluations,
                    run,
                    ExposurePurpose::Diagnosis,
                    Some(DisclosureLevel::RowContent),
                    "workflow follow-up analysis",
                )
                .await?;
                links
            }
            WorkflowStage::StopDecision => {
                let decision = execute_stop_decision(
                    store,
                    definition,
                    run,
                    &history,
                    &usage,
                    &benchmark_authority.development,
                )
                .await?;
                let mut links = vec![
                    link("stop_decision", decision.id, &decision.fingerprint),
                    link(
                        "development_acceptance_assessment",
                        decision.acceptance_assessment_id,
                        &decision.acceptance_assessment_fingerprint,
                    ),
                ];
                if decision.should_continue {
                    links.push(link(
                        "workflow_continue",
                        decision.id,
                        &decision.fingerprint,
                    ));
                }
                links
            }
            WorkflowStage::SealedEvaluation => {
                require_latest_training_check(store, &history, benchmark_authority).await?;
                let suite = benchmark_authority
                    .sealed
                    .as_ref()
                    .context("workflow has no sealed benchmark suite authority")?;
                validate_suite_access(
                    suite,
                    ExposurePurpose::Acceptance,
                    Some(DisclosureLevel::Aggregate),
                )?;
                let checkpoint_id = artifact_id(&history, "iteration_checkpoint")
                    .or_else(|_| artifact_id(&history, "checkpoint"))?;
                let mut evaluation_ids = Vec::new();
                let mut links = Vec::new();
                for cohort in &suite.cohorts {
                    let evaluation = match store
                        .query_evaluation_runs(evaluation_core::ports::EvaluationRunQuery {
                            checkpoint_id: Some(checkpoint_id),
                            snapshot_id: Some(cohort.snapshot_id),
                            state: Some(evaluation_core::domain::EvaluationRunState::Completed),
                            limit: 100,
                            offset: 0,
                        })
                        .await?
                        .into_iter()
                        .find(|value| {
                            value.protocol_fingerprint == cohort.protocol_fingerprint
                                && value.split == cohort.split
                        }) {
                        Some(value) => value,
                        None => {
                            let child = child_execution::reserve_next(
                                store,
                                attempt,
                                WorkflowChildKind::EvaluationRun,
                                cohort.cohort_id.to_string(),
                            )
                            .await?;
                            evaluation::run_workflow(
                                checkpoint_id,
                                cohort.snapshot_id,
                                &cohort.protocol,
                                &child,
                                store.clone(),
                            )
                            .await?
                            .run
                        }
                    };
                    evaluation_ids.push(evaluation.id);
                    links.push(link(
                        "sealed_evaluation_run",
                        evaluation.id,
                        &artifact_core::fingerprint(&evaluation)?,
                    ));
                }
                let mut inputs = Vec::new();
                for cohort in &suite.cohorts {
                    inputs.push(CohortAssessmentInput {
                        cohort_id: cohort.cohort_id,
                        run: Some(load_matching_evaluation(store, cohort, &evaluation_ids).await?),
                        comparison: None,
                    });
                }
                let candidate = assess_benchmark(suite, inputs)?;
                let assessment = match store
                    .query_acceptance_assessments(AcceptanceAssessmentQuery {
                        suite_id: Some(suite.id),
                        checkpoint_id: candidate.checkpoint_id,
                        state: None,
                        limit: 10_000,
                        offset: 0,
                    })
                    .await?
                    .into_iter()
                    .find(|value| value.evaluation_run_ids == candidate.evaluation_run_ids)
                {
                    Some(value) => value,
                    None => {
                        store.create_acceptance_assessment(&candidate).await?;
                        candidate
                    }
                };
                record_suite_exposures(
                    store,
                    suite,
                    evaluation_ids,
                    run,
                    ExposurePurpose::Acceptance,
                    Some(DisclosureLevel::Aggregate),
                    "explicit final sealed assessment",
                )
                .await?;
                links.push(link(
                    "sealed_acceptance_assessment",
                    assessment.id,
                    &assessment.fingerprint,
                ));
                links
            }
            WorkflowStage::Promotion => {
                require_latest_training_check(store, &history, benchmark_authority).await?;
                let existing = store.get_workflow_promotion(run.id).await?;
                let promotion = match existing {
                    Some(value) => value,
                    None => create_promotion(store, definition, run, &history).await?,
                };
                vec![link(
                    "model_promotion",
                    promotion.id,
                    &promotion.fingerprint,
                )]
            }
            other => anyhow::bail!("workflow stage {other:?} is not connected yet"),
        };
        usage.validate_against(&definition.budget)?;
        Ok(StageExecution::completed(artifacts, usage))
    })
}

pub(super) struct StageExecution {
    pub(super) artifacts: Vec<WorkflowArtifactLink>,
    pub(super) usage: WorkflowBudgetUsage,
    pub(super) state: StageAttemptState,
    pub(super) reason: Option<String>,
}

impl StageExecution {
    pub(super) fn completed(
        artifacts: Vec<WorkflowArtifactLink>,
        usage: WorkflowBudgetUsage,
    ) -> Self {
        Self {
            artifacts,
            usage,
            state: StageAttemptState::Completed,
            reason: None,
        }
    }

    pub(super) fn failed(
        artifacts: Vec<WorkflowArtifactLink>,
        usage: WorkflowBudgetUsage,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            artifacts,
            usage,
            state: StageAttemptState::Failed,
            reason: Some(reason.into()),
        }
    }
}

async fn require_stage_training_check(
    store: &SqliteStore,
    history: &[WorkflowStageAttempt],
    check_kind: &str,
    snapshot_kind: &str,
    authority: &WorkflowBenchmarkAuthority,
) -> anyhow::Result<()> {
    let check = artifact_link(history, check_kind)?;
    let snapshot_id = artifact_id(history, snapshot_kind)?;
    training_benchmark_gate::require_linked_clean_check(
        store,
        check.artifact_id,
        &check.artifact_fingerprint,
        snapshot_id,
        authority,
    )
    .await?;
    Ok(())
}

async fn require_latest_training_check(
    store: &SqliteStore,
    history: &[WorkflowStageAttempt],
    authority: &WorkflowBenchmarkAuthority,
) -> anyhow::Result<()> {
    if artifact_link(history, "iteration_checkpoint").is_ok() {
        require_stage_training_check(
            store,
            history,
            "iteration_training_benchmark_check",
            "iteration_snapshot",
            authority,
        )
        .await
    } else {
        require_stage_training_check(
            store,
            history,
            "training_benchmark_check",
            "snapshot",
            authority,
        )
        .await
    }
}
