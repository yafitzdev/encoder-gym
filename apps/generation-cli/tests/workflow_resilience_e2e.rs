pub mod support;

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use dataset_quality_core::policy::{EvaluatorEgressPolicy, QualityPreset};
use generation_supervisor_core::{
    ports::GenerationSupervisorStore,
    preset::{
        EvaluatorProfileRequest, GenerationSupervisionRequest, GeneratorProfileRequest,
        QualityImportance, RepairApprovalRequest, SupervisionLimits, SupervisionQualityLevel,
    },
};
use serde_json::{Value, json};
use synthetic_data_sqlite::SqliteStore;
use workflow_core::{
    execution::WorkflowChildKind,
    ports::{WorkflowRunQuery, WorkflowRunStore},
    workflow::{WorkflowQualityAuthenticity, WorkflowQualityGateRequest, WorkflowRunState},
};

use support::{
    run, run_json,
    workflow_fixture::{GenerationMode, WorkflowFixture},
};

#[test]
fn governed_supervision_pauses_repairs_and_reaches_qualified_training() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sidecar = root.join("adapters/research-agent-pi/dist/main.js");
    if !sidecar.is_file() {
        eprintln!(
            "skipping governed supervisor workflow because {} is not built",
            sidecar.display()
        );
        return;
    }
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("supervised-workflow.toml", |manifest| {
        manifest.workflow.generation_supervision = Some(test_supervision_request());
        manifest.workflow.budget.maximum_cumulative_rows = 256;
        manifest.workflow.budget.maximum_generation_attempts = 512;
        manifest.workflow.budget.maximum_generation_requests = 512;
        manifest.development.contract.metric_requirements[0].minimum = Some(1.0);
    });
    let prepared = run_json(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest_path.to_str().expect("UTF-8 manifest path"),
        ],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let workflow_run_id = string_at(&paused, "/run/id");
    if paused["run"]["state"] != "awaiting_user" {
        let supervisor_run_id = latest_artifact_id(&paused, "generation_supervisor_run");
        let issues = run_json(
            fixture.database_url(),
            ["supervisor", "issues", &supervisor_run_id],
        );
        panic!("supervised workflow did not pause:\n{paused:#}\nissues:\n{issues:#}");
    }
    assert_eq!(paused["latest_attempt"]["stage"], "generation");
    if paused["latest_attempt"]["state"] != "awaiting_user" {
        panic!("supervised generation attempt failed:\n{paused:#}");
    }
    assert_eq!(
        child_execution_count(&paused, "generation_supervisor_run"),
        1
    );
    let supervisor_run_id = latest_artifact_id(&paused, "generation_supervisor_run");

    let repeated = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(repeated["attempt_count"], paused["attempt_count"]);
    assert_eq!(
        repeated["latest_attempt"]["id"],
        paused["latest_attempt"]["id"]
    );

    repair_supervisor(fixture.database_url(), &sidecar, &supervisor_run_id);

    let curation_pause = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(curation_pause["run"]["state"], "awaiting_user");
    assert_eq!(
        curation_pause["latest_attempt"]["stage"], "curation_review",
        "{curation_pause:#}"
    );
    assert_eq!(
        artifact_count(&curation_pause, "supervisor_qualification_handoff"),
        1
    );
    let initial_handoff_id =
        latest_artifact_id(&curation_pause, "supervisor_qualification_handoff");
    assert_eq!(artifact_count(&curation_pause, "quality_report"), 1);
    let proposal_id = latest_artifact_id(&curation_pause, "curation_proposal");
    let approved = run_json(
        fixture.database_url(),
        [
            "quality",
            "manifest-review",
            &proposal_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "admit only the directly qualified supervisor handoff",
        ],
    );
    let manifest_id = string_at(&approved, "/manifest/id");
    let trained = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert!(
        matches!(
            trained["run"]["state"].as_str(),
            Some("awaiting_approval" | "development_complete")
        ),
        "{trained:#}"
    );
    assert_eq!(artifact_count(&trained, "quality_manifest"), 1);
    assert_eq!(artifact_count(&trained, "snapshot"), 1);
    assert_eq!(artifact_count(&trained, "training_run"), 1);
    assert_eq!(artifact_count(&trained, "evaluation_run"), 1);
    let snapshot_id = latest_artifact_id(&trained, "snapshot");
    let snapshot = run_json(fixture.database_url(), ["snapshot", "show", &snapshot_id]);
    assert_eq!(snapshot["qualified"], true);
    assert_eq!(snapshot["manifest_id"], manifest_id);
    let handoff_trace = run_json(
        fixture.database_url(),
        [
            "provenance",
            "supervisor-qualification-handoff",
            &initial_handoff_id,
        ],
    );
    assert_eq!(handoff_trace["kind"], "supervisor_qualification_handoff");
    let handoff_trace = serde_json::to_string(&handoff_trace).expect("handoff provenance JSON");
    assert!(handoff_trace.contains("generation_supervisor_run"));
    assert!(handoff_trace.contains("generation_quality_contract"));
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );

    assert_eq!(
        trained["run"]["state"], "awaiting_approval",
        "supervised acceptance must produce an iteration proposal: {trained:#}"
    );
    {
        let iteration_pause = run_json(
            fixture.database_url(),
            [
                "workflow",
                "approve",
                &workflow_run_id,
                "--note",
                "exercise the bounded supervised data-diff path",
            ],
        );
        assert_eq!(iteration_pause["run"]["state"], "awaiting_user");
        assert_eq!(
            iteration_pause["latest_attempt"]["stage"], "dataset_diff_generation",
            "{iteration_pause:#}"
        );
        assert_eq!(
            iteration_pause["latest_attempt"]["state"], "awaiting_user",
            "{iteration_pause:#}"
        );
        let iteration_supervisor_run_id =
            latest_artifact_id(&iteration_pause, "generation_supervisor_run");
        assert_ne!(iteration_supervisor_run_id, supervisor_run_id);

        let unchanged = run_json(
            fixture.database_url(),
            ["workflow", "resume", &workflow_run_id],
        );
        assert_eq!(unchanged["attempt_count"], iteration_pause["attempt_count"]);
        repair_supervisor(
            fixture.database_url(),
            &sidecar,
            &iteration_supervisor_run_id,
        );

        let iteration_curation = run_json(
            fixture.database_url(),
            ["workflow", "resume", &workflow_run_id],
        );
        assert_eq!(iteration_curation["run"]["state"], "awaiting_user");
        assert_eq!(
            iteration_curation["latest_attempt"]["stage"], "iteration_curation_review",
            "{iteration_curation:#}"
        );
        assert_eq!(
            artifact_count(&iteration_curation, "supervisor_qualification_handoff"),
            2
        );
        let proposal_id = latest_artifact_id(&iteration_curation, "iteration_curation_proposal");
        let approved = run_json(
            fixture.database_url(),
            [
                "quality",
                "manifest-review",
                &proposal_id,
                "--approve",
                "--reviewer",
                "workflow-test",
                "--reason",
                "admit the directly qualified iteration diff",
            ],
        );
        let iteration_manifest_id = string_at(&approved, "/manifest/id");
        let iterated = run_json(
            fixture.database_url(),
            ["workflow", "resume", &workflow_run_id],
        );
        assert!(
            matches!(
                iterated["run"]["state"].as_str(),
                Some("development_complete" | "awaiting_approval")
            ),
            "{iterated:#}"
        );
        assert_eq!(artifact_count(&iterated, "iteration_snapshot"), 1);
        assert_eq!(artifact_count(&iterated, "iteration_training_run"), 1);
        assert_eq!(artifact_count(&iterated, "iteration_evaluation_run"), 1);
        let iteration_snapshot_id = latest_artifact_id(&iterated, "iteration_snapshot");
        let iteration_snapshot = run_json(
            fixture.database_url(),
            ["snapshot", "show", &iteration_snapshot_id],
        );
        assert_eq!(iteration_snapshot["qualified"], true);
        assert_eq!(iteration_snapshot["manifest_id"], iteration_manifest_id);
        let supervisor_children = iterated["child_executions"]
            .as_array()
            .expect("workflow child executions")
            .iter()
            .filter(|child| child["child_kind"] == "generation_supervisor_run")
            .collect::<Vec<_>>();
        assert_eq!(supervisor_children.len(), 4, "{iterated:#}");
        assert_eq!(
            supervisor_children
                .iter()
                .map(|child| child["child_execution_id"]
                    .as_str()
                    .expect("supervisor child ID"))
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            2,
            "each supervised stage must link one stable child across its pause and completion attempts"
        );
        let provenance = run_json(
            fixture.database_url(),
            ["provenance", "workflow-run", &workflow_run_id],
        );
        let provenance = serde_json::to_string(&provenance).expect("workflow provenance JSON");
        for kind in [
            "generation_quality_contract",
            "generation_supervisor_run",
            "supervisor_qualification_handoff",
            "supervisor_qualification_application",
        ] {
            assert!(
                provenance.contains(kind),
                "supervised workflow provenance omitted {kind}: {provenance}"
            );
        }
        assert_eq!(
            run_json(fixture.database_url(), ["doctor"])["healthy"],
            true
        );
    }
}

#[test]
fn governed_supervision_cancellation_targets_the_exact_linked_run() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("supervised-cancellation.toml", |manifest| {
        manifest.workflow.generation_supervision = Some(test_supervision_request());
        manifest.workflow.budget.maximum_cumulative_rows = 256;
        manifest.workflow.budget.maximum_generation_attempts = 512;
        manifest.workflow.budget.maximum_generation_requests = 512;
    });
    let prepared = run_json(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest_path.to_str().expect("UTF-8 manifest path"),
        ],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    assert_eq!(paused["run"]["state"], "awaiting_user", "{paused:#}");
    let workflow_run_id = string_at(&paused, "/run/id");
    let supervisor_run_id = latest_artifact_id(&paused, "generation_supervisor_run");

    let requested = run_json(
        fixture.database_url(),
        ["workflow", "cancel", &workflow_run_id],
    );
    assert_eq!(requested["cancel_requested"], true);
    let supervisor = run_json(
        fixture.database_url(),
        ["supervisor", "status", &supervisor_run_id],
    );
    assert_eq!(supervisor["run"]["id"], supervisor_run_id);
    let cancellation_requested = tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            SqliteStore::connect(fixture.database_url())
                .await
                .expect("database connects")
                .supervisor_cancel_requested(
                    uuid::Uuid::parse_str(&supervisor_run_id).expect("supervisor UUID"),
                )
                .await
                .expect("cancellation lookup")
        });
    assert_eq!(cancellation_requested, Some(true));

    let cancelled = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(cancelled["run"]["state"], "cancelled");
    assert_eq!(cancelled["latest_attempt"]["state"], "cancelled");
    assert_eq!(
        child_execution_count(&cancelled, "generation_supervisor_run"),
        1
    );
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

fn test_supervision_request() -> GenerationSupervisionRequest {
    GenerationSupervisionRequest {
        quality_level: SupervisionQualityLevel::Balanced,
        authenticity_importance: QualityImportance::Off,
        diversity_importance: QualityImportance::Normal,
        maximum_prompt_repairs: 2,
        monitoring_rows_per_scope: 1,
        limits: SupervisionLimits {
            maximum_generated_rows: 128,
            maximum_evaluator_requests: 64,
            maximum_evaluator_tokens: 1_000_000,
            maximum_pi_tokens: 100_000,
            maximum_duration_seconds: 600,
            maximum_retries_per_external_call: 2,
            maximum_cost_microunits: None,
        },
        repair_approval: RepairApprovalRequest::Manual,
        generator: GeneratorProfileRequest {
            batch_size: 8,
            max_retries: 0,
            max_attempt_multiplier: 2,
            ..GeneratorProfileRequest::default()
        },
        evaluator: EvaluatorProfileRequest::default(),
    }
}

fn repair_supervisor(database_url: &str, sidecar: &std::path::Path, supervisor_run_id: &str) {
    let temporary = tempfile::tempdir().expect("diagnosis directory");
    let script = temporary.path().join("diagnosis.json");
    std::fs::write(
        &script,
        serde_json::to_vec_pretty(&supervisor_diagnosis_script()).unwrap(),
    )
    .unwrap();
    let diagnosed = run_json(
        database_url,
        [
            "supervisor",
            "diagnose",
            supervisor_run_id,
            "--script",
            script.to_str().expect("UTF-8 script path"),
            "--pi-sidecar",
            sidecar.to_str().expect("UTF-8 sidecar path"),
        ],
    );
    let session_id = string_at(&diagnosed, "/session/id");
    let reviewed = run_json(
        database_url,
        [
            "supervisor",
            "revision-review",
            supervisor_run_id,
            &session_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "approve a bounded guidance-only repair",
        ],
    );
    assert_eq!(reviewed["status"]["state"], "canary");
    let canary = run_json(database_url, ["supervisor", "canary", supervisor_run_id]);
    assert_eq!(canary["status"]["state"], "running", "{canary:#}");
}

#[test]
fn quality_gated_workflow_pauses_for_the_exact_latest_manifest_and_trains_qualified_data() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("quality-gated-workflow.toml", |manifest| {
        manifest.workflow.quality_gate = Some(WorkflowQualityGateRequest {
            preset: QualityPreset::Fast,
            egress_policy: EvaluatorEgressPolicy::LocalOnly,
            authenticity: WorkflowQualityAuthenticity::Off,
            maximum_cost_microusd: None,
            evaluator_backend: "deterministic-fake".into(),
            evaluator_protocol_version: "quality-evaluator-v1".into(),
        });
    });
    let manifest_path = manifest_path.to_str().expect("UTF-8 manifest path");
    let prepared = run_json(
        fixture.database_url(),
        ["project", "prepare", manifest_path],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");

    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let workflow_run_id = string_at(&paused, "/run/id");
    assert_eq!(paused["run"]["state"], "awaiting_user");
    assert_eq!(
        paused["latest_attempt"]["stage"], "curation_review",
        "quality-gated workflow did not reach its review pause: {paused}"
    );
    assert_eq!(
        paused["latest_attempt"]["state"], "awaiting_user",
        "quality-gated workflow did not persist its review pause: {paused}"
    );
    assert_eq!(artifact_count(&paused, "quality_audit_plan"), 1);
    assert_eq!(artifact_count(&paused, "quality_audit_run"), 1);
    assert_eq!(artifact_count(&paused, "quality_report"), 1);
    let initial_proposal_id = latest_artifact_id(&paused, "curation_proposal");
    let audit_run_id = latest_artifact_id(&paused, "quality_audit_run");

    let repeated = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(repeated["attempt_count"], paused["attempt_count"]);
    assert_eq!(
        repeated["latest_attempt"]["id"],
        paused["latest_attempt"]["id"]
    );
    assert_eq!(
        latest_artifact_id(&repeated, "curation_proposal"),
        initial_proposal_id
    );

    // Approve the displayed proposal, then append newer row reviews. Resume
    // must materialize a successor and refuse the now-stale manifest.
    let stale_approval = run_json(
        fixture.database_url(),
        [
            "quality",
            "manifest-review",
            &initial_proposal_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "approve before the later append-only row reviews",
        ],
    );
    let stale_manifest_id = string_at(&stale_approval, "/manifest/id");
    let assessments = run_json(
        fixture.database_url(),
        ["quality", "assessments", &audit_run_id, "--limit", "100"],
    );
    let assessments = assessments.as_array().expect("quality assessments");
    assert!(!assessments.is_empty());
    for assessment in assessments {
        let assessment_id = string_at(assessment, "/id");
        run_json(
            fixture.database_url(),
            [
                "quality",
                "row-review",
                &assessment_id,
                "--include",
                "--reviewer",
                "workflow-test",
                "--reason",
                "include this row in the workflow training snapshot",
            ],
        );
    }

    let refreshed_pause = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(refreshed_pause["run"]["state"], "awaiting_user");
    assert_eq!(
        refreshed_pause["latest_attempt"]["stage"],
        "curation_review"
    );
    assert_eq!(artifact_count(&refreshed_pause, "snapshot"), 0);
    let successor_proposal_id = latest_artifact_id(&refreshed_pause, "curation_proposal");
    assert_ne!(successor_proposal_id, initial_proposal_id);
    assert_eq!(
        latest_artifact_id(&refreshed_pause, "curation_proposal"),
        successor_proposal_id
    );
    assert!(
        !refreshed_pause["attempts"]
            .as_array()
            .expect("attempts")
            .iter()
            .flat_map(|attempt| attempt["artifacts"].as_array().expect("artifacts"))
            .any(|artifact| artifact["kind"] == "quality_manifest"
                && artifact["artifact_id"] == stale_manifest_id),
        "a stale approved manifest must not enter workflow history"
    );

    let approved = run_json(
        fixture.database_url(),
        [
            "quality",
            "manifest-review",
            &successor_proposal_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "approve the exact latest reviewed proposal",
        ],
    );
    let manifest_id = string_at(&approved, "/manifest/id");
    let resumed = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert!(matches!(
        resumed["run"]["state"].as_str(),
        Some("awaiting_approval" | "development_complete")
    ));
    assert_eq!(artifact_count(&resumed, "quality_manifest"), 1);
    assert_eq!(artifact_count(&resumed, "curation_application"), 1);
    assert_eq!(artifact_count(&resumed, "snapshot"), 1);
    assert_eq!(artifact_count(&resumed, "training_run"), 1);
    let child_kinds = resumed["child_executions"]
        .as_array()
        .expect("workflow child executions")
        .iter()
        .map(|child| child["child_kind"].as_str().expect("child kind"))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(child_kinds.contains("generation_job"));
    assert!(child_kinds.contains("quality_audit_run"));
    assert!(child_kinds.contains("training_run"));
    assert!(child_kinds.contains("evaluation_run"));
    let snapshot_id = latest_artifact_id(&resumed, "snapshot");
    let shown = run_json(fixture.database_url(), ["snapshot", "show", &snapshot_id]);
    assert_eq!(shown["qualified"], true);
    assert_eq!(shown["manifest_id"], manifest_id);
    let qualified_member_ids = run_json(
        fixture.database_url(),
        ["snapshot", "members", &snapshot_id],
    )
    .as_array()
    .expect("qualified snapshot members")
    .iter()
    .map(|member| string_at(member, "/source_row_id"))
    .collect::<std::collections::BTreeSet<_>>();
    for evidence_snapshot_id in [fixture.development_snapshot_id, fixture.sealed_snapshot_id] {
        let evidence_member_ids = run_json(
            fixture.database_url(),
            ["snapshot", "members", &evidence_snapshot_id.to_string()],
        )
        .as_array()
        .expect("evaluation snapshot members")
        .iter()
        .map(|member| string_at(member, "/source_row_id"))
        .collect::<std::collections::BTreeSet<_>>();
        assert!(
            qualified_member_ids.is_disjoint(&evidence_member_ids),
            "qualified training data must be disjoint from every evaluation cohort"
        );
    }
    let provenance = run_json(
        fixture.database_url(),
        ["provenance", "workflow-run", &workflow_run_id],
    );
    let provenance = serde_json::to_string(&provenance).expect("workflow provenance JSON");
    assert!(provenance.contains("child_executions"));
    for kind in [
        "quality_audit_plan",
        "quality_audit_run",
        "dataset_quality_report",
        "curation_proposal",
        "curation_manifest_review",
        "approved_curation_manifest",
        "curation_application",
    ] {
        assert!(
            provenance.contains(kind),
            "quality-gated workflow provenance omitted {kind}: {provenance}"
        );
    }
}

#[test]
fn same_name_unqualified_snapshot_cannot_satisfy_a_workflow_quality_gate() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("quality-gated-collision.toml", |manifest| {
        manifest.workflow.quality_gate = Some(WorkflowQualityGateRequest {
            preset: QualityPreset::Fast,
            egress_policy: EvaluatorEgressPolicy::LocalOnly,
            authenticity: WorkflowQualityAuthenticity::Off,
            maximum_cost_microusd: None,
            evaluator_backend: "deterministic-fake".into(),
            evaluator_protocol_version: "quality-evaluator-v1".into(),
        });
    });
    let manifest_path = manifest_path.to_str().expect("UTF-8 manifest path");
    let prepared = run_json(
        fixture.database_url(),
        ["project", "prepare", manifest_path],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let dataset_id = string_at(&prepared, "/preparation/dataset_id");
    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let workflow_run_id = string_at(&paused, "/run/id");
    let proposal_id = latest_artifact_id(&paused, "curation_proposal");
    run_json(
        fixture.database_url(),
        [
            "quality",
            "manifest-review",
            &proposal_id,
            "--approve",
            "--reviewer",
            "workflow-test",
            "--reason",
            "approve the exact displayed proposal",
        ],
    );

    let colliding_name = format!("baseline-workflow-{workflow_run_id}-0");
    run_json(
        fixture.database_url(),
        [
            "snapshot",
            "create",
            &dataset_id,
            "--name",
            &colliding_name,
            "--train-ratio",
            "0.8",
            "--validation-ratio",
            "0.0",
            "--test-ratio",
            "0.2",
            "--seed",
            "42",
        ],
    );
    let rejected = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(rejected["run"]["state"], "awaiting_user");
    assert_eq!(rejected["latest_attempt"]["stage"], "snapshot");
    assert_eq!(rejected["latest_attempt"]["state"], "failed");
    assert!(
        rejected["latest_attempt"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("not qualified by the exact approved manifest"))
    );
    assert_eq!(artifact_count(&rejected, "training_run"), 0);
}

#[test]
fn quality_gated_workflow_can_be_cancelled_while_waiting_for_manifest_approval() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("quality-gated-cancellation.toml", |manifest| {
        manifest.workflow.quality_gate = Some(WorkflowQualityGateRequest {
            preset: QualityPreset::Fast,
            egress_policy: EvaluatorEgressPolicy::LocalOnly,
            authenticity: WorkflowQualityAuthenticity::Off,
            maximum_cost_microusd: None,
            evaluator_backend: "deterministic-fake".into(),
            evaluator_protocol_version: "quality-evaluator-v1".into(),
        });
    });
    let prepared = run_json(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest_path.to_str().expect("UTF-8 manifest path"),
        ],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let paused = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let workflow_run_id = string_at(&paused, "/run/id");
    assert_eq!(paused["latest_attempt"]["stage"], "curation_review");
    assert_eq!(paused["latest_attempt"]["state"], "awaiting_user");

    let requested = run_json(
        fixture.database_url(),
        ["workflow", "cancel", &workflow_run_id],
    );
    assert_eq!(requested["cancel_requested"], true);
    let cancelled = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id],
    );
    assert_eq!(cancelled["run"]["state"], "cancelled");
    assert_eq!(cancelled["latest_attempt"]["state"], "cancelled");
    assert_eq!(artifact_count(&cancelled, "snapshot"), 0);
    assert_eq!(artifact_count(&cancelled, "training_run"), 0);
}

#[test]
fn interrupted_workflow_resumes_once_and_then_stays_idempotent() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let initialized = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id, "--initialize-only"],
    );
    let run_id = string_at(&initialized, "/run/id");
    assert_eq!(initialized["attempt"]["stage"], "initial_allocation");

    let recovery = run_json(fixture.database_url(), ["recovery", "scan"]);
    assert!(
        recovery
            .as_array()
            .expect("recovery list")
            .iter()
            .any(|record| record["workflow_kind"] == "encoder_workflow"
                && record["workflow_id"] == run_id)
    );

    let resumed = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert!(matches!(
        resumed["run"]["state"].as_str(),
        Some("awaiting_approval" | "development_complete")
    ));
    assert_eq!(artifact_count(&resumed, "initial_allocation"), 1);
    assert_eq!(artifact_count(&resumed, "generation_plan"), 1);
    let training_run_id = latest_artifact_id(&resumed, "training_run");
    let training_check_id = latest_artifact_id(&resumed, "training_benchmark_check");
    let training_run = run_json(
        fixture.database_url(),
        ["training", "status", &training_run_id],
    );
    assert_eq!(
        training_run["input_binding"]["authority_id"],
        training_check_id
    );
    assert_eq!(
        training_run["input_binding"]["authority_kind"],
        "training_benchmark_check"
    );
    assert!(training_run["input_binding"]["input_fingerprint"].is_string());

    let attempt_count = resumed["attempt_count"].clone();
    let latest_attempt_id = resumed["latest_attempt"]["id"].clone();
    let repeated = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(repeated["attempt_count"], attempt_count);
    assert_eq!(repeated["latest_attempt"]["id"], latest_attempt_id);
    assert_eq!(artifact_count(&repeated, "initial_allocation"), 1);
    assert_eq!(artifact_count(&repeated, "generation_plan"), 1);

    let unresolved = run_json(fixture.database_url(), ["recovery", "list"]);
    assert!(
        !unresolved
            .as_array()
            .expect("recovery list")
            .iter()
            .any(|record| record["workflow_kind"] == "encoder_workflow"
                && record["workflow_id"] == run_id)
    );
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

#[test]
fn benchmark_overlap_blocks_before_any_training_run_and_persists_the_check() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest_path = fixture.write_variant("leakage-firewall.toml", |manifest| {
        manifest.project.snapshot.train_ratio = 1.0;
        manifest.project.snapshot.validation_ratio = 0.0;
        manifest.project.snapshot.test_ratio = 0.0;
    });
    let prepared = run_json(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest_path.to_str().expect("UTF-8 manifest path"),
        ],
    );
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let dataset_id = string_at(&prepared, "/preparation/dataset_id");
    let input_path = manifest_path
        .parent()
        .expect("manifest directory")
        .join("contaminated-training.jsonl");
    std::fs::write(
        &input_path,
        concat!(
            r#"{"text":"Why was I charged twice?","label":"billing","difficulty":"easy","writing_style":"clean","ambiguity":"obvious"}"#,
            "\n"
        ),
    )
    .expect("contaminated input writes");
    let imported = run_json(
        fixture.database_url(),
        [
            "dataset",
            "import",
            &dataset_id,
            "--input",
            input_path.to_str().expect("UTF-8 input path"),
            "--format",
            "jsonl",
            "--dimension",
            "difficulty=difficulty",
            "--dimension",
            "writing_style=writing_style",
            "--dimension",
            "ambiguity=ambiguity",
        ],
    );
    assert_eq!(imported["accepted_rows"], 1);

    let blocked = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    assert_eq!(blocked["run"]["state"], "failed");
    assert_eq!(blocked["latest_attempt"]["stage"], "training");
    assert_eq!(blocked["latest_attempt"]["state"], "failed");
    assert_eq!(blocked["latest_attempt"]["retryable"], false);
    assert!(
        blocked["latest_attempt"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("blocked before backend startup"))
    );
    assert_eq!(artifact_count(&blocked, "training_benchmark_check"), 1);
    assert_eq!(artifact_count(&blocked, "training_run"), 0);
    assert_eq!(artifact_count(&blocked, "checkpoint"), 0);

    let check_id = latest_artifact_id(&blocked, "training_benchmark_check");
    let check = run_json(
        fixture.database_url(),
        ["benchmark", "training-check-show", &check_id],
    );
    assert_eq!(check["status"], "blocked");
    assert!(
        check["training_member_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    let validation = run_json(
        fixture.database_url(),
        ["benchmark", "training-check-validate", &check_id],
    );
    assert_eq!(validation["valid"], true);
    assert_eq!(validation["training_allowed"], false);
    assert!(
        validation["reasons"]
            .as_array()
            .is_some_and(|reasons| !reasons.is_empty())
    );
    let report_id = string_at(&check, "/contamination_report_id");
    let forbidden_override = run(
        fixture.database_url(),
        [
            "contamination",
            "override",
            &report_id,
            "--reason",
            "must remain forbidden",
            "--approved-by",
            "workflow-test",
        ],
    );
    assert!(!forbidden_override.status.success());
    assert!(
        String::from_utf8_lossy(&forbidden_override.stderr)
            .contains("training-benchmark contamination reports cannot be overridden")
    );
    let listed = run_json(
        fixture.database_url(),
        [
            "benchmark",
            "training-check-list",
            "--snapshot-id",
            string_at(&check, "/training_snapshot_id").as_str(),
        ],
    );
    assert_eq!(listed.as_array().expect("check list").len(), 1);
    let trace = run_json(
        fixture.database_url(),
        ["provenance", "training-benchmark-check", &check_id],
    );
    let parent_kinds = trace["parents"]
        .as_array()
        .expect("check provenance parents")
        .iter()
        .map(|parent| parent["kind"].as_str().expect("parent kind"))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        parent_kinds,
        std::collections::BTreeSet::from(["snapshot", "benchmark_bundle", "contamination_report",])
    );
}

#[test]
fn cancellation_is_persisted_before_work_starts_and_resume_is_safe() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let initialized = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id, "--initialize-only"],
    );
    let run_id = string_at(&initialized, "/run/id");

    let requested = run_json(fixture.database_url(), ["workflow", "cancel", &run_id]);
    assert_eq!(requested["cancel_requested"], true);
    let cancelled = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(cancelled["run"]["state"], "cancelled");
    assert_eq!(cancelled["latest_attempt"]["state"], "cancelled");
    assert_eq!(artifact_count(&cancelled, "generation_job"), 0);

    let attempt_count = cancelled["attempt_count"].clone();
    let repeated = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(repeated["run"]["state"], "cancelled");
    assert_eq!(repeated["attempt_count"], attempt_count);
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

#[test]
fn cancellation_targets_the_exact_active_generation_child() {
    let server = BlockingFailingServer::start();
    let fixture = WorkflowFixture::new(GenerationMode::OpenAiCompatible {
        base_url: server.base_url(),
        model: "blocking-generation".into(),
    });
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let process = Command::new(env!("CARGO_BIN_EXE_synth"))
        .args([
            "--database-url",
            fixture.database_url(),
            "--output",
            "json",
            "workflow",
            "start",
            &definition_id,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("workflow process starts");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !server.request_seen.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "generation request did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let (workflow_run_id, job_id) = runtime.block_on(async {
        let store = SqliteStore::connect(fixture.database_url())
            .await
            .expect("database connects");
        let run = store
            .query_workflow_runs(WorkflowRunQuery {
                definition_id: None,
                state: Some(WorkflowRunState::Running),
                limit: 10,
                offset: 0,
            })
            .await
            .expect("running workflows query")
            .into_iter()
            .next()
            .expect("running workflow");
        let attempt_id = run.latest_attempt_id.expect("running attempt");
        let child = store
            .list_workflow_child_executions(attempt_id)
            .await
            .expect("child executions")
            .into_iter()
            .find(|child| child.child_kind == WorkflowChildKind::GenerationJob)
            .expect("generation child");
        (run.id, child.child_execution_id)
    });

    let requested = run_json(
        fixture.database_url(),
        ["workflow", "cancel", &workflow_run_id.to_string()],
    );
    assert_eq!(requested["cancel_requested"], true);
    let child = run_json(
        fixture.database_url(),
        ["job", "status", &job_id.to_string()],
    );
    assert_eq!(child["id"], job_id.to_string());
    assert_eq!(child["cancel_requested"], true);

    server.release.store(true, Ordering::Release);
    let output = process.wait_with_output().expect("workflow process exits");
    assert!(
        output.status.success(),
        "workflow process failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let status: Value = serde_json::from_slice(&output.stdout).expect("workflow status JSON");
    assert_eq!(status["run"]["state"], "cancelled");
    assert_eq!(status["latest_attempt"]["state"], "cancelled");
    assert_eq!(child_execution_count(&status, "generation_job"), 1);
}

#[test]
fn interrupted_generation_resumes_the_exact_reserved_child_identity() {
    let server = BlockingFailingServer::start();
    let fixture = WorkflowFixture::new(GenerationMode::OpenAiCompatible {
        base_url: server.base_url(),
        model: "interrupted-generation".into(),
    });
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let mut process = Command::new(env!("CARGO_BIN_EXE_synth"))
        .args([
            "--database-url",
            fixture.database_url(),
            "--output",
            "json",
            "workflow",
            "start",
            &definition_id,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("workflow process starts");
    let deadline = Instant::now() + Duration::from_secs(15);
    while !server.request_seen.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "generation request did not start"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let (workflow_run_id, job_id) =
        running_child_ids(fixture.database_url(), WorkflowChildKind::GenerationJob);
    process.kill().expect("workflow process is killed");
    process.wait().expect("killed workflow process exits");
    server.release.store(true, Ordering::Release);

    let recovery = run_json(fixture.database_url(), ["recovery", "scan"]);
    assert!(
        recovery
            .as_array()
            .expect("recovery records")
            .iter()
            .any(|record| record["workflow_kind"] == "encoder_workflow"
                && record["workflow_id"] == workflow_run_id.to_string())
    );
    assert!(
        recovery
            .as_array()
            .expect("recovery records")
            .iter()
            .any(|record| record["workflow_kind"] == "generation"
                && record["workflow_id"] == job_id.to_string())
    );

    let resumed = run_json(
        fixture.database_url(),
        ["workflow", "resume", &workflow_run_id.to_string()],
    );
    assert_eq!(child_execution_count(&resumed, "generation_job"), 1);
    assert_eq!(
        resumed["child_executions"]
            .as_array()
            .expect("child executions")[0]["child_execution_id"],
        job_id.to_string()
    );
    let jobs = run_json(
        fixture.database_url(),
        [
            "job",
            "list",
            "--plan-id",
            &latest_artifact_id(&resumed, "generation_plan"),
        ],
    );
    assert_eq!(jobs.as_array().expect("generation jobs").len(), 1);
}

#[test]
fn failing_backend_retries_are_bounded_and_append_only() {
    let server = AlwaysFailingServer::start();
    let fixture = WorkflowFixture::new(GenerationMode::OpenAiCompatible {
        base_url: server.base_url(),
        model: "always-fails".into(),
    });
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let dataset_id = string_at(&prepared, "/preparation/dataset_id");

    let first = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    let run_id = string_at(&first, "/run/id");
    assert_failed_generation_attempt(&first, 1, true);
    assert_eq!(child_execution_count(&first, "generation_job"), 1);

    let second = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_failed_generation_attempt(&second, 2, false);
    assert_eq!(child_execution_count(&second, "generation_job"), 2);
    let mut failed_attempts = generation_failures(&second);
    failed_attempts.sort_by_key(|attempt| attempt["attempt"].as_u64());
    assert_eq!(failed_attempts.len(), 2);
    let retry_start = second["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .find(|attempt| {
            attempt["stage"] == "generation"
                && attempt["attempt"] == 2
                && attempt["state"] == "running"
        })
        .expect("persisted retry start");
    assert_eq!(retry_start["predecessor_id"], failed_attempts[0]["id"]);
    assert_eq!(failed_attempts[1]["predecessor_id"], retry_start["id"]);

    let attempt_count = second["attempt_count"].clone();
    let repeated = run_json(fixture.database_url(), ["workflow", "resume", &run_id]);
    assert_eq!(repeated["attempt_count"], attempt_count);
    assert_eq!(generation_failures(&repeated).len(), 2);

    let rows = run_json(
        fixture.database_url(),
        ["rows", "--dataset-id", &dataset_id, "--status", "accepted"],
    );
    assert_eq!(rows, serde_json::json!([]));
    let jobs = run_json(
        fixture.database_url(),
        ["job", "list", "--dataset-id", &dataset_id],
    );
    assert_eq!(jobs.as_array().expect("jobs").len(), 2);
    assert!(
        jobs.as_array()
            .expect("jobs")
            .iter()
            .all(|job| job["state"] == "failed")
    );
    assert_eq!(
        run_json(fixture.database_url(), ["doctor"])["healthy"],
        true
    );
}

#[test]
#[ignore = "requires an explicitly configured live endpoint and may incur provider cost"]
fn live_openai_compatible_smoke_is_explicitly_opt_in() {
    let base_url = std::env::var("SYNTH_E2E_OPENAI_BASE_URL")
        .expect("set SYNTH_E2E_OPENAI_BASE_URL before running the ignored smoke test");
    let model = std::env::var("SYNTH_E2E_OPENAI_MODEL")
        .expect("set SYNTH_E2E_OPENAI_MODEL before running the ignored smoke test");
    assert!(
        std::env::var_os("SYNTH_OPENAI_API_KEY").is_some(),
        "set SYNTH_OPENAI_API_KEY before running the ignored smoke test"
    );
    let fixture = WorkflowFixture::new(GenerationMode::OpenAiCompatible { base_url, model });
    let prepared = fixture.prepare();
    let definition_id = string_at(&prepared, "/preparation/workflow_definition_id");
    let result = run_json(
        fixture.database_url(),
        ["workflow", "start", &definition_id],
    );
    assert!(
        artifact_count(&result, "generation_job") > 0,
        "live workflow produced no generation job: {}",
        live_failure_summary(&result)
    );
    assert_ne!(
        result["run"]["state"],
        "failed",
        "live workflow failed: {}",
        live_failure_summary(&result)
    );
}

fn live_failure_summary(status: &Value) -> String {
    let state = status["run"]["state"].as_str().unwrap_or("unknown");
    let stage = status["latest_attempt"]["stage"]
        .as_str()
        .unwrap_or("unknown");
    let reason = status["latest_attempt"]["reason"]
        .as_str()
        .unwrap_or("no stage reason");
    let job_error = status["generation"]
        .as_array()
        .and_then(|generations| generations.last())
        .and_then(|generation| generation["jobs"].as_array())
        .and_then(|jobs| jobs.last())
        .and_then(|job| job["error_message"].as_str())
        .unwrap_or("no generation job error");
    format!("state={state}, stage={stage}, reason={reason}, job_error={job_error}")
}

fn assert_failed_generation_attempt(status: &Value, attempt: u64, retryable: bool) {
    assert_eq!(
        status["run"]["state"],
        if retryable { "awaiting_user" } else { "failed" }
    );
    assert_eq!(status["latest_attempt"]["stage"], "generation");
    assert_eq!(status["latest_attempt"]["state"], "failed");
    assert_eq!(status["latest_attempt"]["attempt"], attempt);
    assert_eq!(status["latest_attempt"]["retryable"], retryable);
}

fn generation_failures(status: &Value) -> Vec<&Value> {
    status["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .filter(|attempt| attempt["stage"] == "generation" && attempt["state"] == "failed")
        .collect()
}

fn artifact_count(status: &Value, kind: &str) -> usize {
    status["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .flat_map(|attempt| attempt["artifacts"].as_array().expect("attempt artifacts"))
        .filter(|artifact| artifact["kind"] == kind)
        .count()
}

fn child_execution_count(status: &Value, kind: &str) -> usize {
    status["child_executions"]
        .as_array()
        .expect("workflow child executions")
        .iter()
        .filter(|child| child["child_kind"] == kind)
        .count()
}

fn running_child_ids(database_url: &str, kind: WorkflowChildKind) -> (uuid::Uuid, uuid::Uuid) {
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let store = SqliteStore::connect(database_url)
                .await
                .expect("database connects");
            let run = store
                .query_workflow_runs(WorkflowRunQuery {
                    definition_id: None,
                    state: Some(WorkflowRunState::Running),
                    limit: 10,
                    offset: 0,
                })
                .await
                .expect("running workflows query")
                .into_iter()
                .next()
                .expect("running workflow");
            let child = store
                .list_workflow_child_executions(run.latest_attempt_id.expect("running attempt"))
                .await
                .expect("child executions")
                .into_iter()
                .find(|child| child.child_kind == kind)
                .expect("requested child kind");
            (run.id, child.child_execution_id)
        })
}

fn latest_artifact_id(status: &Value, kind: &str) -> String {
    status["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .rev()
        .flat_map(|attempt| attempt["artifacts"].as_array().expect("attempt artifacts"))
        .find(|artifact| artifact["kind"] == kind)
        .and_then(|artifact| artifact["artifact_id"].as_str())
        .unwrap_or_else(|| panic!("workflow has no {kind} artifact: {status}"))
        .to_owned()
}

fn string_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {value}"))
        .to_owned()
}

fn supervisor_diagnosis_script() -> Value {
    json!([{
        "text": "Inspect the bounded evidence, preview one narrow repair, submit it, and finish.",
        "toolCalls": [
            {"name": "inspect_quality_contract", "arguments": {}},
            {"name": "inspect_quality_window", "arguments": {}},
            {"name": "inspect_failure_breakdown", "arguments": {}},
            {"name": "inspect_current_prompt_guidance", "arguments": {}},
            {"name": "preview_prompt_revision", "arguments": {
                "replacement_guidance": ["Vary phrasing and add concrete situational detail without naming the label."],
                "expected_improvements": [{"metric": "qualified_rate", "minimum_delta_basis_points": 1500}]
            }},
            {"name": "submit_prompt_revision", "arguments": {
                "cause": "repetition_mode_collapse",
                "summary": "The current guidance permits shortcut-heavy synthetic templates.",
                "replacement_guidance": ["Vary phrasing and add concrete situational detail without naming the label."],
                "expected_improvements": [{"metric": "qualified_rate", "minimum_delta_basis_points": 1500}]
            }},
            {"name": "finish_supervision", "arguments": {
                "outcome": "revision_submitted",
                "summary": "A bounded guidance-only repair is ready for operator review."
            }}
        ]
    }])
}

struct AlwaysFailingServer {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct BlockingFailingServer {
    address: std::net::SocketAddr,
    stop: Arc<AtomicBool>,
    request_seen: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl BlockingFailingServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server binds");
        let address = listener.local_addr().expect("mock server address");
        listener
            .set_nonblocking(true)
            .expect("mock server becomes nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let request_seen = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_seen = Arc::clone(&request_seen);
        let thread_release = Arc::clone(&release);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut request = [0_u8; 8192];
                        let _ = stream.read(&mut request);
                        thread_seen.store(true, Ordering::Release);
                        while !thread_release.load(Ordering::Acquire)
                            && !thread_stop.load(Ordering::Relaxed)
                        {
                            thread::sleep(Duration::from_millis(5));
                        }
                        if !thread_stop.load(Ordering::Relaxed) {
                            fail_request_without_read(&mut stream);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("mock server accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            request_seen,
            release,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}/v1", self.address)
    }
}

impl Drop for BlockingFailingServer {
    fn drop(&mut self) {
        self.release.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("mock server stops");
        }
    }
}

impl AlwaysFailingServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock server binds");
        let address = listener.local_addr().expect("mock server address");
        listener
            .set_nonblocking(true)
            .expect("mock server becomes nonblocking");
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => fail_request(&mut stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("mock server accept failed: {error}"),
                }
            }
        });
        Self {
            address,
            stop,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> String {
        format!("http://{}/v1", self.address)
    }
}

impl Drop for AlwaysFailingServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            thread.join().expect("mock server stops");
        }
    }
}

fn fail_request(stream: &mut TcpStream) {
    let mut request = [0_u8; 8192];
    let _ = stream.read(&mut request);
    fail_request_without_read(stream);
}

fn fail_request_without_read(stream: &mut TcpStream) {
    let _ = stream.write_all(
            b"HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        );
}
