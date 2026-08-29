use std::collections::BTreeSet;

use dataset_core::domain::SnapshotSplit;
use evaluation_core::domain::EvaluationProtocol;
use project_config::{GenerationBackendKind, ProjectOverrides, TrainingBackendKind};
use thiserror::Error;
use workflow_core::{
    allocation::{InitialAllocationRequest, allocate_initial_budget},
    benchmark::{
        BenchmarkCohortEvidence, BenchmarkCohortRequest, BenchmarkSuiteKind, BenchmarkSuiteRequest,
        build_benchmark_suite,
    },
    contamination::{
        CohortContaminationInput, ContaminationMember, ContaminationReport, ContaminationStatus,
        check_contamination,
    },
    governance::{CohortRole, CohortRoleDecision, DisclosureLevel, EvaluationCohort},
    workflow::{
        IterationGovernance, WorkflowDefinition, WorkflowDefinitionRequest,
        WorkflowInitialAllocation, WorkflowStage,
    },
};

use crate::{
    CohortEvidence, CohortManifest, ContaminationPreview, PreparationBundle, PreparationEvidence,
    PreparationIssue, PreparationManifest, PreparationPreview, PreparedProject, SuiteManifest,
    domain::{CellTargetPreview, prepared_fingerprint},
};

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PreparationError {
    #[error("unsupported preparation manifest version: {0}")]
    Version(u32),
    #[error("invalid preparation manifest: {0}")]
    Invalid(String),
    #[error("missing immutable snapshot evidence: {0}")]
    MissingEvidence(uuid::Uuid),
    #[error("preparation is blocked: {0}")]
    Blocked(String),
    #[error("could not construct preparation artifacts: {0}")]
    Domain(String),
}

pub fn preview_project(
    manifest: &PreparationManifest,
    evidence: &PreparationEvidence,
) -> Result<PreparationPreview, PreparationError> {
    let context = build_context(manifest, evidence)?;
    let issues = context.issues();
    let eligible = issues.is_empty();
    Ok(PreparationPreview {
        manifest_fingerprint: manifest
            .fingerprint()
            .map_err(|error| PreparationError::Domain(error.to_string()))?,
        project_name: manifest.name.trim().to_owned(),
        dataset_name: context.resolved.dataset.name.clone(),
        generation_backend: generation_backend_name(context.resolved.generation.backend).into(),
        generation_model: context.resolved.generation.model.clone(),
        training_backend: training_backend_name(context.resolved.training.backend).into(),
        requested_total_rows: context.allocation.requested_total_rows,
        initial_target_rows: context.allocation.initial_target_rows,
        reserved_rows: context.allocation.reserved_rows,
        estimated_initial_requests: context
            .allocation
            .cells
            .iter()
            .map(|cell| {
                u64::from(cell.additional_required)
                    .div_ceil(u64::from(context.resolved.generation.batch_size))
            })
            .sum(),
        cells: context
            .allocation
            .cells
            .iter()
            .map(|cell| CellTargetPreview {
                cell: cell.cell.clone(),
                target: cell.target,
            })
            .collect(),
        development_cohorts: manifest.development.cohorts.len(),
        sealed_cohorts: manifest
            .sealed
            .as_ref()
            .map_or(0, |suite| suite.cohorts.len()),
        contamination: context.contamination_previews(),
        stages: workflow_stages(
            manifest.workflow.advisor.is_some(),
            manifest.sealed.is_some(),
        ),
        governance_mode: match manifest.workflow.governance {
            IterationGovernance::ReviewEachIteration => "review_each_iteration",
            IterationGovernance::PreauthorizedBounded { .. } => "preauthorized_bounded",
        }
        .into(),
        issues,
        eligible,
    })
}

pub fn compile_project(
    manifest: &PreparationManifest,
    evidence: &PreparationEvidence,
) -> Result<PreparationBundle, PreparationError> {
    let context = build_context(manifest, evidence)?;
    let issues = context.issues();
    if !issues.is_empty() {
        return Err(PreparationError::Blocked(
            issues
                .into_iter()
                .map(|issue| issue.message)
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }

    let default_generation_plan = context
        .resolved
        .generation_plan(&context.dataset)
        .map_err(domain)?;
    let project_configuration = context
        .resolved
        .persisted(context.dataset.id, default_generation_plan.id)
        .map_err(domain)?;
    let development_report = context
        .suite_reports
        .first()
        .cloned()
        .ok_or_else(|| PreparationError::Invalid("development suite is required".into()))?;
    let development_suite = build_suite(
        &manifest.development,
        BenchmarkSuiteKind::Development,
        &context,
        &development_report,
    )?;
    let sealed_suite = manifest
        .sealed
        .as_ref()
        .zip(context.suite_reports.get(1))
        .map(|(suite, report)| {
            build_suite(
                suite,
                BenchmarkSuiteKind::SealedAcceptance,
                &context,
                report,
            )
        })
        .transpose()?;
    let workflow_definition = WorkflowDefinition::new(WorkflowDefinitionRequest {
        name: manifest.workflow.name.clone(),
        dataset_id: context.dataset.id,
        project_configuration_id: project_configuration.id,
        project_configuration_fingerprint: project_configuration.fingerprint.clone(),
        development_suite_id: development_suite.id,
        development_suite_fingerprint: development_suite.fingerprint.clone(),
        sealed_suite_id: sealed_suite.as_ref().map(|suite| suite.id),
        sealed_suite_fingerprint: sealed_suite.as_ref().map(|suite| suite.fingerprint.clone()),
        initial_allocation: WorkflowInitialAllocation {
            total_rows: manifest.workflow.total_rows,
            reserved_rows: manifest.workflow.reserved_rows,
            policy: manifest.workflow.allocation_policy.clone(),
            constraints: manifest.workflow.allocation_constraints.clone(),
        },
        analysis_protocol: manifest.workflow.analysis_protocol.clone(),
        optimization_protocol: manifest.workflow.optimization_protocol.clone(),
        advisor: manifest.workflow.advisor.clone(),
        training_iteration_policy: manifest.workflow.training_iteration_policy,
        governance: manifest.workflow.governance.clone(),
        budget: manifest.workflow.budget.clone(),
        policy: manifest.workflow.policy.clone(),
    })
    .map_err(domain)?;
    let manifest_fingerprint = manifest
        .fingerprint()
        .map_err(|error| PreparationError::Domain(error.to_string()))?;
    let mut preparation = PreparedProject {
        id: uuid::Uuid::new_v4(),
        name: manifest.name.trim().to_owned(),
        manifest_fingerprint,
        dataset_id: context.dataset.id,
        project_configuration_id: project_configuration.id,
        development_suite_id: development_suite.id,
        sealed_suite_id: sealed_suite.as_ref().map(|suite| suite.id),
        workflow_definition_id: workflow_definition.id,
        created_at: chrono::Utc::now(),
        fingerprint: String::new(),
    };
    preparation.fingerprint = prepared_fingerprint(&preparation).map_err(domain)?;

    Ok(PreparationBundle {
        dataset: context.dataset,
        default_generation_plan,
        backend_configuration: context.resolved.backend_configuration(),
        project_configuration,
        cohorts: context
            .cohorts
            .iter()
            .map(|cohort| cohort.cohort.clone())
            .collect(),
        role_decisions: context
            .cohorts
            .iter()
            .map(|cohort| cohort.role.clone())
            .collect(),
        contamination_reports: std::iter::once(context.global_report)
            .chain(context.suite_reports)
            .collect(),
        development_suite,
        sealed_suite,
        workflow_definition,
        preparation,
    })
}

struct PreparedCohort {
    manifest: CohortManifest,
    cohort: EvaluationCohort,
    role: CohortRoleDecision,
    evidence: CohortEvidence,
}

struct BuildContext {
    resolved: project_config::ResolvedProjectConfig,
    dataset: generation_core::domain::DatasetDefinition,
    allocation: workflow_core::allocation::InitialAllocationResult,
    cohorts: Vec<PreparedCohort>,
    global_report: ContaminationReport,
    suite_reports: Vec<ContaminationReport>,
}

impl BuildContext {
    fn issues(&self) -> Vec<PreparationIssue> {
        let mut issues = self
            .allocation
            .issues
            .iter()
            .map(|issue| PreparationIssue {
                code: "allocation_infeasible".into(),
                message: format!("{issue:?}"),
            })
            .collect::<Vec<_>>();
        for (scope, report) in std::iter::once(("all_cohorts", &self.global_report)).chain(
            self.suite_reports
                .iter()
                .enumerate()
                .map(|(index, report)| (if index == 0 { "development" } else { "sealed" }, report)),
        ) {
            if report.status == ContaminationStatus::Blocked {
                issues.extend(report.reasons.iter().map(|reason| PreparationIssue {
                    code: "contamination_blocked".into(),
                    message: format!("{scope}: {reason}"),
                }));
            }
        }
        issues
    }

    fn contamination_previews(&self) -> Vec<ContaminationPreview> {
        std::iter::once(("all_cohorts", &self.global_report))
            .chain(
                self.suite_reports
                    .iter()
                    .enumerate()
                    .map(|(index, report)| {
                        (if index == 0 { "development" } else { "sealed" }, report)
                    }),
            )
            .map(|(scope, report)| ContaminationPreview {
                scope: scope.into(),
                status: report.status,
                counts: report.counts.clone(),
                reasons: report.reasons.clone(),
            })
            .collect()
    }
}

fn build_context(
    manifest: &PreparationManifest,
    evidence: &PreparationEvidence,
) -> Result<BuildContext, PreparationError> {
    validate_manifest_shape(manifest)?;
    let resolved = manifest
        .project
        .clone()
        .resolve(ProjectOverrides::default())
        .map_err(domain)?;
    let dataset = resolved.dataset_definition().map_err(domain)?;
    let total_rows = u32::try_from(manifest.workflow.total_rows)
        .map_err(|_| PreparationError::Invalid("workflow.total_rows exceeds u32".into()))?;
    let reserved_rows = u32::try_from(manifest.workflow.reserved_rows)
        .map_err(|_| PreparationError::Invalid("workflow.reserved_rows exceeds u32".into()))?;
    let allocation = allocate_initial_budget(
        &dataset,
        InitialAllocationRequest {
            total_rows,
            reserved_rows,
            policy: manifest.workflow.allocation_policy.clone(),
            current_coverage: Vec::new(),
            constraints: manifest.workflow.allocation_constraints.clone(),
        },
    )
    .map_err(domain)?;

    let mut seen = BTreeSet::new();
    let manifests = manifest
        .development
        .cohorts
        .iter()
        .chain(manifest.sealed.iter().flat_map(|suite| &suite.cohorts));
    let mut cohorts = Vec::new();
    for cohort_manifest in manifests {
        if !seen.insert((cohort_manifest.snapshot_id, cohort_manifest.split)) {
            return Err(PreparationError::Invalid(format!(
                "snapshot {} split {} is assigned more than once",
                cohort_manifest.snapshot_id,
                cohort_manifest.split.as_str()
            )));
        }
        let cohort_evidence = evidence
            .cohorts
            .get(&cohort_manifest.snapshot_id)
            .ok_or(PreparationError::MissingEvidence(
                cohort_manifest.snapshot_id,
            ))?
            .clone();
        validate_evidence(cohort_manifest, &cohort_evidence, &resolved.dataset.labels)?;
        let cohort = EvaluationCohort::new(
            &cohort_manifest.name,
            cohort_evidence.snapshot.id,
            &cohort_evidence.snapshot.fingerprint,
            cohort_manifest.split,
            cohort_manifest.origin,
        )
        .map_err(domain)?;
        let role = CohortRoleDecision::initial(
            &cohort,
            cohort_manifest.role,
            "assigned by preparation manifest",
        )
        .map_err(domain)?;
        cohorts.push(PreparedCohort {
            manifest: cohort_manifest.clone(),
            cohort,
            role,
            evidence: cohort_evidence,
        });
    }
    let contamination_inputs = cohorts
        .iter()
        .map(|cohort| {
            contamination_input(cohort, manifest.contamination.group_dimension.as_deref())
        })
        .collect();
    let global_report = check_contamination(
        contamination_inputs,
        manifest.contamination.group_dimension.clone(),
        manifest.contamination.policy.clone(),
    )
    .map_err(domain)?;
    let suite_reports = std::iter::once(&manifest.development)
        .chain(manifest.sealed.iter())
        .map(|suite| {
            let names = suite
                .cohorts
                .iter()
                .map(|cohort| cohort.name.as_str())
                .collect::<BTreeSet<_>>();
            let inputs = cohorts
                .iter()
                .filter(|cohort| names.contains(cohort.manifest.name.as_str()))
                .map(|cohort| {
                    contamination_input(cohort, manifest.contamination.group_dimension.as_deref())
                })
                .collect();
            check_contamination(
                inputs,
                manifest.contamination.group_dimension.clone(),
                manifest.contamination.policy.clone(),
            )
            .map_err(domain)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(BuildContext {
        resolved,
        dataset,
        allocation,
        cohorts,
        global_report,
        suite_reports,
    })
}

fn validate_manifest_shape(manifest: &PreparationManifest) -> Result<(), PreparationError> {
    if manifest.version != 1 {
        return Err(PreparationError::Version(manifest.version));
    }
    if manifest.name.trim().is_empty()
        || manifest.workflow.name.trim().is_empty()
        || manifest.development.name.trim().is_empty()
    {
        return Err(PreparationError::Invalid(
            "project, workflow, and development suite names must not be empty".into(),
        ));
    }
    if manifest.development.cohorts.is_empty()
        || manifest
            .sealed
            .as_ref()
            .is_some_and(|suite| suite.cohorts.is_empty() || suite.name.trim().is_empty())
    {
        return Err(PreparationError::Invalid(
            "each configured suite requires a name and at least one cohort".into(),
        ));
    }
    let mut names = BTreeSet::new();
    for cohort in manifest
        .development
        .cohorts
        .iter()
        .chain(manifest.sealed.iter().flat_map(|suite| &suite.cohorts))
    {
        if cohort.name.trim().is_empty() || !names.insert(cohort.name.trim()) {
            return Err(PreparationError::Invalid(
                "cohort names must be non-empty and globally unique".into(),
            ));
        }
    }
    if manifest.workflow.policy.enable_advisor != manifest.workflow.advisor.is_some() {
        return Err(PreparationError::Invalid(
            "workflow advisor presence must match policy.enable_advisor".into(),
        ));
    }
    Ok(())
}

fn validate_evidence(
    manifest: &CohortManifest,
    evidence: &CohortEvidence,
    labels: &[String],
) -> Result<(), PreparationError> {
    if evidence.snapshot.id != manifest.snapshot_id
        || evidence.snapshot.source_dataset_id != evidence.source_dataset.id
    {
        return Err(PreparationError::Invalid(format!(
            "snapshot evidence identity mismatch for {}",
            manifest.name
        )));
    }
    if evidence.source_dataset.labels != labels {
        return Err(PreparationError::Invalid(format!(
            "snapshot {} label order does not match the project dataset",
            manifest.snapshot_id
        )));
    }
    let selected = evidence
        .members
        .iter()
        .filter(|member| member.split == manifest.split)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return Err(PreparationError::Invalid(format!(
            "snapshot {} has no members in split {}",
            manifest.snapshot_id,
            manifest.split.as_str()
        )));
    }
    if evidence
        .members
        .iter()
        .any(|member| member.snapshot_id != evidence.snapshot.id)
    {
        return Err(PreparationError::Invalid(format!(
            "snapshot {} contains a foreign member",
            manifest.snapshot_id
        )));
    }
    if manifest.role == CohortRole::SealedAcceptance
        && (manifest.disclosure != DisclosureLevel::Aggregate || manifest.adaptation_eligible)
    {
        return Err(PreparationError::Invalid(
            "sealed cohorts must be aggregate-only and adaptation-ineligible".into(),
        ));
    }
    Ok(())
}

fn contamination_input(
    value: &PreparedCohort,
    group_dimension: Option<&str>,
) -> CohortContaminationInput {
    CohortContaminationInput {
        cohort: value.cohort.clone(),
        role: value.role.clone(),
        members: value
            .evidence
            .members
            .iter()
            .filter(|member| member.split == value.manifest.split)
            .map(|member| ContaminationMember::from_snapshot_member(member, group_dimension))
            .collect(),
    }
}

fn build_suite(
    manifest: &SuiteManifest,
    kind: BenchmarkSuiteKind,
    context: &BuildContext,
    report: &ContaminationReport,
) -> Result<workflow_core::benchmark::BenchmarkSuite, PreparationError> {
    let names = manifest
        .cohorts
        .iter()
        .map(|cohort| cohort.name.as_str())
        .collect::<BTreeSet<_>>();
    let selected = context
        .cohorts
        .iter()
        .filter(|cohort| names.contains(cohort.manifest.name.as_str()))
        .collect::<Vec<_>>();
    let requests = selected
        .iter()
        .map(|cohort| BenchmarkCohortRequest {
            cohort_id: cohort.cohort.id,
            protocol: cohort
                .manifest
                .protocol
                .clone()
                .unwrap_or_else(|| default_protocol(context, cohort.manifest.split)),
            disclosure: cohort.manifest.disclosure,
            adaptation_eligible: cohort.manifest.adaptation_eligible,
        })
        .collect();
    let evidence = selected
        .into_iter()
        .map(|cohort| BenchmarkCohortEvidence {
            cohort: cohort.cohort.clone(),
            role: cohort.role.clone(),
        })
        .collect();
    build_benchmark_suite(
        BenchmarkSuiteRequest {
            name: manifest.name.clone(),
            kind,
            task: context.resolved.dataset.task.clone(),
            labels: context.resolved.dataset.labels.clone(),
            required_model_formats: manifest.required_model_formats.clone(),
            cohorts: requests,
            contract: manifest.contract.clone(),
        },
        evidence,
        report,
        None,
    )
    .map_err(domain)
}

fn default_protocol(context: &BuildContext, split: SnapshotSplit) -> EvaluationProtocol {
    let configured = &context.resolved.evaluation;
    EvaluationProtocol {
        split,
        batch_size: configured.batch_size,
        top_k: configured.top_k.clone(),
        calibration_bins: configured.calibration_bins,
        minimum_slice_support: configured.minimum_slice_support,
        dimension_intersections: configured.dimension_intersections.clone(),
        bootstrap_samples: configured.bootstrap_samples,
        statistical_seed: configured.statistical_seed,
        confidence_level: configured.confidence_level,
    }
}

fn workflow_stages(has_advisor: bool, has_sealed_suite: bool) -> Vec<WorkflowStage> {
    let mut stages = vec![
        WorkflowStage::InitialAllocation,
        WorkflowStage::Generation,
        WorkflowStage::Snapshot,
        WorkflowStage::Training,
        WorkflowStage::DevelopmentEvaluation,
        WorkflowStage::AcceptanceAssessment,
        WorkflowStage::ErrorAnalysis,
    ];
    if has_advisor {
        stages.push(WorkflowStage::Advisor);
    }
    stages.extend([
        WorkflowStage::OptimizationProposal,
        WorkflowStage::Approval,
        WorkflowStage::ProposalApplication,
        WorkflowStage::DatasetDiffGeneration,
        WorkflowStage::IterationSnapshot,
        WorkflowStage::IterationTraining,
        WorkflowStage::IterationEvaluation,
        WorkflowStage::Comparison,
        WorkflowStage::FollowupAnalysis,
        WorkflowStage::StopDecision,
    ]);
    if has_sealed_suite {
        stages.extend([WorkflowStage::SealedEvaluation, WorkflowStage::Promotion]);
    }
    stages
}

const fn generation_backend_name(value: GenerationBackendKind) -> &'static str {
    match value {
        GenerationBackendKind::Fake => "fake",
        GenerationBackendKind::OpenaiCompatible => "openai-compatible",
    }
}

const fn training_backend_name(value: TrainingBackendKind) -> &'static str {
    match value {
        TrainingBackendKind::HashingLinear => "hashing-linear",
        TrainingBackendKind::BertCpu => "bert-cpu",
    }
}

fn domain(error: impl std::fmt::Display) -> PreparationError {
    PreparationError::Domain(error.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use dataset_core::domain::{
        DatasetSnapshot, SnapshotMember, SnapshotSplit, SourceProvenance, SplitConfiguration,
        SplitRatios,
    };
    use generation_core::domain::DatasetDefinition;
    use project_config::{ProjectConfig, ProjectOverrides};
    use uuid::Uuid;
    use workflow_core::{
        allocation::InitialAllocationPolicy,
        benchmark::AcceptanceContract,
        contamination::{ContaminationKind, ContaminationStatus},
        governance::{CohortOrigin, CohortRole, DisclosureLevel},
        workflow::{IterationGovernance, WorkflowBudget, WorkflowPolicy},
    };

    use crate::{
        CohortEvidence, CohortManifest, ContaminationManifest, PreparationError,
        PreparationEvidence, PreparationManifest, SuiteManifest, WorkflowManifest, compile_project,
        preview_project,
    };

    const PROJECT: &str = r#"
version = 1

[dataset]
name = "support"
task = "Classify support requests"
labels = ["billing", "fraud"]

[[dataset.dimensions]]
name = "style"
values = ["clean", "messy"]

[generation]
target_per_cell = 10
batch_size = 20
"#;

    #[test]
    fn preview_is_stable_and_conserves_the_exact_initial_budget() {
        let (manifest, evidence) = fixture();

        let first = preview_project(&manifest, &evidence).expect("preview");
        let second = preview_project(&manifest, &evidence).expect("preview again");

        assert_eq!(first, second);
        assert!(first.eligible);
        assert_eq!(first.requested_total_rows, 100);
        assert_eq!(first.initial_target_rows, 80);
        assert_eq!(first.reserved_rows, 20);
        assert_eq!(first.cells.iter().map(|cell| cell.target).sum::<u32>(), 80);
        assert_eq!(first.cells.len(), 4);
        assert_eq!(first.estimated_initial_requests, 4);
        assert_eq!(first.contamination[0].status, ContaminationStatus::Clean);
    }

    #[test]
    fn compiles_an_artifact_bundle_with_resolved_references() {
        let (manifest, evidence) = fixture();

        let bundle = compile_project(&manifest, &evidence).expect("compile");

        assert_eq!(bundle.project_configuration.dataset_id, bundle.dataset.id);
        assert_eq!(
            bundle.project_configuration.generation_plan_id,
            bundle.default_generation_plan.id
        );
        assert_eq!(
            bundle.workflow_definition.project_configuration_id,
            bundle.project_configuration.id
        );
        assert_eq!(
            bundle.workflow_definition.development_suite_id,
            bundle.development_suite.id
        );
        assert_eq!(
            bundle.preparation.workflow_definition_id,
            bundle.workflow_definition.id
        );
        assert_eq!(
            bundle
                .preparation
                .reproduce_fingerprint()
                .expect("fingerprint"),
            bundle.preparation.fingerprint
        );
    }

    #[test]
    fn rejects_unknown_manifest_fields() {
        let (manifest, _) = fixture();
        let mut value = serde_json::to_value(manifest).expect("serialize");
        value
            .as_object_mut()
            .expect("object")
            .insert("surprise".into(), serde_json::json!(true));

        let error = PreparationManifest::parse_json(&value.to_string()).expect_err("reject");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn rejects_duplicate_snapshot_split_assignments() {
        let (mut manifest, evidence) = fixture();
        let mut repeated = manifest.development.cohorts[0].clone();
        repeated.name = "repeat".into();
        repeated.role = CohortRole::SealedAcceptance;
        repeated.disclosure = DisclosureLevel::Aggregate;
        repeated.adaptation_eligible = false;
        manifest.sealed = Some(SuiteManifest {
            name: "sealed".into(),
            required_model_formats: Vec::new(),
            cohorts: vec![repeated],
            contract: empty_contract(),
        });

        let error = preview_project(&manifest, &evidence).expect_err("reject");

        assert!(
            matches!(error, PreparationError::Invalid(message) if message.contains("assigned more than once"))
        );
    }

    #[test]
    fn rejects_label_order_mismatch_and_empty_selected_split() {
        let (manifest, mut evidence) = fixture();
        let snapshot_id = manifest.development.cohorts[0].snapshot_id;
        evidence
            .cohorts
            .get_mut(&snapshot_id)
            .expect("evidence")
            .source_dataset
            .labels
            .reverse();
        assert!(matches!(
            preview_project(&manifest, &evidence),
            Err(PreparationError::Invalid(message)) if message.contains("label order")
        ));

        let (mut manifest, evidence) = fixture();
        manifest.development.cohorts[0].split = SnapshotSplit::Validation;
        assert!(matches!(
            preview_project(&manifest, &evidence),
            Err(PreparationError::Invalid(message)) if message.contains("no members")
        ));
    }

    #[test]
    fn blocks_cross_cohort_contamination() {
        let (mut manifest, mut evidence) = fixture();
        let first = manifest.development.cohorts[0].clone();
        let original = evidence
            .cohorts
            .get(&first.snapshot_id)
            .expect("first evidence")
            .clone();
        let second_snapshot_id = Uuid::new_v4();
        let mut second_evidence = original;
        second_evidence.snapshot.id = second_snapshot_id;
        second_evidence.snapshot.name = "diagnostic snapshot".into();
        second_evidence.snapshot.fingerprint = "sha256:diagnostic".into();
        for member in &mut second_evidence.members {
            member.id = Uuid::new_v4();
            member.snapshot_id = second_snapshot_id;
        }
        evidence.cohorts.insert(second_snapshot_id, second_evidence);
        manifest.development.cohorts.push(CohortManifest {
            name: "diagnostic".into(),
            snapshot_id: second_snapshot_id,
            split: SnapshotSplit::Test,
            origin: CohortOrigin::InternalSnapshot,
            role: CohortRole::Diagnostic,
            protocol: None,
            disclosure: DisclosureLevel::Predictions,
            adaptation_eligible: true,
        });

        let preview = preview_project(&manifest, &evidence).expect("preview");

        assert!(!preview.eligible);
        assert_eq!(
            preview.contamination[0].status,
            ContaminationStatus::Blocked
        );
        assert!(preview.contamination[0].counts[&ContaminationKind::SourceRow] > 0);
        assert!(matches!(
            compile_project(&manifest, &evidence),
            Err(PreparationError::Blocked(_))
        ));
    }

    #[test]
    fn rejects_unsafe_sealed_disclosure() {
        let (mut manifest, evidence) = fixture();
        manifest.development.cohorts[0].role = CohortRole::SealedAcceptance;
        manifest.development.cohorts[0].disclosure = DisclosureLevel::RowContent;

        assert!(matches!(
            preview_project(&manifest, &evidence),
            Err(PreparationError::Invalid(message)) if message.contains("aggregate-only")
        ));
    }

    fn fixture() -> (PreparationManifest, PreparationEvidence) {
        let project = ProjectConfig::parse(PROJECT).expect("project config");
        let resolved = project
            .clone()
            .resolve(ProjectOverrides::default())
            .expect("resolve");
        let source_dataset = resolved.dataset_definition().expect("dataset");
        let snapshot_id = Uuid::new_v4();
        let source_row_id = Uuid::new_v4();
        let now = Utc::now();
        let snapshot = DatasetSnapshot {
            id: snapshot_id,
            source_dataset_id: source_dataset.id,
            name: "development snapshot".into(),
            description: None,
            split_configuration: SplitConfiguration::new(
                SplitRatios::new(0.8, 0.1, 0.1).expect("ratios"),
                42,
            ),
            member_count: 1,
            fingerprint: "sha256:development".into(),
            created_at: now,
        };
        let member = SnapshotMember {
            id: Uuid::new_v4(),
            snapshot_id,
            source_row_id,
            split: SnapshotSplit::Test,
            text: "Why was I charged twice?".into(),
            label: "billing".into(),
            dimensions: BTreeMap::from([("style".into(), "clean".into())]),
            source_provenance: SourceProvenance::Imported {
                import_id: Uuid::new_v4(),
                source_path: "benchmark.jsonl".into(),
                source_row_number: 1,
            },
            source_created_at: now,
        };
        let manifest = PreparationManifest {
            version: 1,
            name: "support encoder".into(),
            project,
            contamination: ContaminationManifest::default(),
            development: SuiteManifest {
                name: "development".into(),
                required_model_formats: Vec::new(),
                cohorts: vec![CohortManifest {
                    name: "development test".into(),
                    snapshot_id,
                    split: SnapshotSplit::Test,
                    origin: CohortOrigin::InternalSnapshot,
                    role: CohortRole::Development,
                    protocol: None,
                    disclosure: DisclosureLevel::Predictions,
                    adaptation_eligible: true,
                }],
                contract: empty_contract(),
            },
            sealed: None,
            workflow: WorkflowManifest {
                name: "support workflow".into(),
                total_rows: 100,
                reserved_rows: 20,
                allocation_policy: InitialAllocationPolicy::Balanced,
                allocation_constraints: Vec::new(),
                analysis_protocol: None,
                optimization_protocol: None,
                advisor: None,
                training_iteration_policy: None,
                governance: IterationGovernance::ReviewEachIteration,
                budget: WorkflowBudget {
                    maximum_iterations: 2,
                    maximum_initial_rows: 100,
                    maximum_cumulative_rows: 120,
                    maximum_generation_attempts: 500,
                    maximum_generation_requests: 100,
                    maximum_advisor_calls: 0,
                    maximum_advisor_tokens: None,
                    maximum_stage_attempts: 3,
                },
                policy: WorkflowPolicy {
                    minimum_improvement: 0.01,
                    maximum_tolerated_regression: 0.01,
                    stop_on_inconclusive: true,
                    stop_on_invalid: true,
                    enable_advisor: false,
                    require_fresh_development_cohort_after_iterations: None,
                },
            },
        };
        (
            manifest,
            PreparationEvidence {
                cohorts: BTreeMap::from([(
                    snapshot_id,
                    CohortEvidence {
                        snapshot,
                        source_dataset,
                        members: vec![member],
                    },
                )]),
            },
        )
    }

    fn empty_contract() -> AcceptanceContract {
        AcceptanceContract {
            metric_requirements: Vec::new(),
            regression: None,
        }
    }

    #[allow(dead_code)]
    fn _assert_dataset_type(_: DatasetDefinition) {}
}
