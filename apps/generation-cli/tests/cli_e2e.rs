use std::path::Path;

pub mod support;

use optimization_core::protocol::{
    OptimizationProtocol, RecommendationKind, TrainingCandidateRequest,
};
use serde_json::Value;
use workflow_core::promotion::ModelPromotion;

use support::{run, run_json, run_text};

#[test]
fn complete_local_cli_workflow_is_scriptable_and_deterministic() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("workflow.db");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let artifacts = directory.path().join("artifacts");
    let config_path = directory.path().join("project.toml");
    std::fs::write(&config_path, config(&artifacts)).expect("write config");
    let csv_path = directory.path().join("ambiguous.csv");
    std::fs::write(&csv_path, ambiguous_csv()).expect("write CSV");
    let optimization_protocol_path = directory.path().join("optimization.json");
    std::fs::write(
        &optimization_protocol_path,
        serde_json::to_vec_pretty(&OptimizationProtocol::legacy(2, 1))
            .expect("serialize optimization protocol"),
    )
    .expect("write optimization protocol");

    let doctor = run_json(
        &database_url,
        ["doctor", "--config", path(&config_path), "--check-backend"],
    );
    assert_eq!(doctor["healthy"], true);

    let initialized = run_json(&database_url, ["config", "init", path(&config_path)]);
    let dataset_id = string_at(&initialized, "/dataset/id");
    let initial_plan_id = string_at(&initialized, "/generation_plan/id");

    let imported = run_json(
        &database_url,
        [
            "dataset",
            "import",
            &dataset_id,
            "--input",
            path(&csv_path),
            "--format",
            "csv",
            "--batch-size",
            "3",
        ],
    );
    assert_eq!(imported["accepted_rows"], 12);
    assert_eq!(imported["rejected_rows"], 0);

    let generated = run_json(
        &database_url,
        [
            "generate",
            &initial_plan_id,
            "--config",
            path(&config_path),
            "--batch-size",
            "1",
        ],
    );
    assert_eq!(generated["state"], "completed");
    assert_eq!(generated["accepted_rows"], 2);
    let generation_job_id = string_at(&generated, "/id");
    let execution = run_json(&database_url, ["job", "execution", &generation_job_id]);
    assert_eq!(execution["job_id"], generation_job_id);
    assert!(string_at(&execution, "/fingerprint").starts_with("sha256:"));
    assert_eq!(execution["policy"]["batch_size"], 1);
    let attempts = run_json(&database_url, ["job", "attempts", &generation_job_id]);
    assert_eq!(attempts.as_array().expect("attempt array").len(), 2);
    assert!(
        attempts
            .as_array()
            .expect("attempt array")
            .iter()
            .all(|attempt| {
                attempt["state"] == "succeeded"
                    && attempt["outcome_fingerprint"]
                        .as_str()
                        .is_some_and(|fingerprint| fingerprint.starts_with("sha256:"))
            })
    );
    let prompt = run_json(
        &database_url,
        [
            "job",
            "prompt",
            &generation_job_id,
            "--cell-index",
            "0",
            "--requested-count",
            "1",
        ],
    );
    assert_eq!(prompt["execution_fingerprint"], execution["fingerprint"]);
    assert_eq!(prompt["request"]["requested_count"], 1);
    assert!(
        prompt["request"]["user_prompt"]
            .as_str()
            .expect("user prompt")
            .contains("Classify intentionally ambiguous support messages")
    );

    let snapshot_result = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &dataset_id,
            "--config",
            path(&config_path),
        ],
    );
    let snapshot_id = string_at(&snapshot_result, "/snapshot/id");
    assert!(string_at(&snapshot_result, "/snapshot/fingerprint").starts_with("sha256:"));

    let training = run_json(
        &database_url,
        [
            "training",
            "run",
            &snapshot_id,
            "--config",
            path(&config_path),
        ],
    );
    assert_eq!(training["run"]["state"], "completed");
    let training_run_id = string_at(&training, "/run/id");
    let checkpoint_id = training["checkpoints"]
        .as_array()
        .expect("checkpoint array")
        .iter()
        .find(|checkpoint| checkpoint["is_final"] == true)
        .and_then(|checkpoint| checkpoint["id"].as_str())
        .expect("final checkpoint")
        .to_owned();
    let training_choices_path = directory.path().join("training-choices.json");
    let training_space_path = directory.path().join("training-space.json");
    let mut candidate_configuration = training["run"]["configuration"].clone();
    let baseline_epochs = candidate_configuration["epochs"]
        .as_u64()
        .expect("baseline epochs");
    candidate_configuration["epochs"] = serde_json::json!(baseline_epochs + 1);
    std::fs::write(
        &training_choices_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "choices": [{
                "configuration": candidate_configuration,
                "transformer_configuration": null
            }]
        }))
        .expect("serialize training choices"),
    )
    .expect("write training choices");
    let training_space = run_json(
        &database_url,
        [
            "optimize",
            "training-space",
            &training_run_id,
            &checkpoint_id,
            "--choices",
            path(&training_choices_path),
            "--file",
            path(&training_space_path),
        ],
    );
    assert_eq!(training_space["choices"], 1);
    assert!(string_at(&training_space, "/fingerprint").starts_with("sha256:"));

    let evaluation = run_json(
        &database_url,
        [
            "evaluation",
            "run",
            &checkpoint_id,
            "--config",
            path(&config_path),
        ],
    );
    assert_eq!(evaluation["run"]["state"], "completed");
    let evaluation_id = string_at(&evaluation, "/run/id");
    assert!(
        evaluation["run"]["metrics"]["overall"]["accuracy"]
            .as_f64()
            .expect("accuracy")
            < 1.0,
        "ambiguous identical token features must retain at least one error"
    );

    let repeated_evaluation = run_json(
        &database_url,
        [
            "evaluation",
            "run",
            &checkpoint_id,
            "--config",
            path(&config_path),
        ],
    );
    let repeated_evaluation_id = string_at(&repeated_evaluation, "/run/id");
    let comparison = run_json(
        &database_url,
        [
            "evaluation",
            "compare",
            &evaluation_id,
            &repeated_evaluation_id,
        ],
    );
    let comparison_id = string_at(&comparison, "/id");

    // Development evidence must not be a split of the growing generation
    // dataset. Rebuilding a stratified snapshot after adding rows can move a
    // previously held-out source row across a split boundary, which the
    // training-benchmark gate correctly blocks. Keep the benchmark population
    // independent so this E2E exercises iteration rather than accidental
    // holdout reuse.
    let development_csv_path = directory.path().join("development.csv");
    std::fs::write(&development_csv_path, development_csv()).expect("write development CSV");
    let development_dataset = run_json(
        &database_url,
        [
            "dataset",
            "create",
            "--name",
            "development-support",
            "--task",
            "Classify intentionally ambiguous support messages.",
            "--label",
            "billing",
            "--label",
            "fraud",
        ],
    );
    let development_dataset_id = string_at(&development_dataset, "/id");
    let development_import = run_json(
        &database_url,
        [
            "dataset",
            "import",
            &development_dataset_id,
            "--input",
            path(&development_csv_path),
            "--format",
            "csv",
        ],
    );
    assert_eq!(development_import["accepted_rows"], 12);
    let development_snapshot = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &development_dataset_id,
            "--name",
            "development-evidence-only",
            "--train-ratio",
            "0",
            "--validation-ratio",
            "0",
            "--test-ratio",
            "1",
            "--seed",
            "13",
        ],
    );
    let development_snapshot_id = string_at(&development_snapshot, "/snapshot/id");
    let development_evaluation = run_json(
        &database_url,
        [
            "evaluation",
            "run",
            &checkpoint_id,
            "--snapshot-id",
            &development_snapshot_id,
            "--split",
            "test",
            "--config",
            path(&config_path),
        ],
    );
    assert!(
        development_evaluation["run"]["metrics"]["overall"]["accuracy"]
            .as_f64()
            .expect("development accuracy")
            < 1.0,
        "independent ambiguous development examples must trigger an iteration"
    );
    let development_evaluation_id = string_at(&development_evaluation, "/run/id");
    let development_cohort = run_json(
        &database_url,
        [
            "cohort",
            "create",
            &development_snapshot_id,
            "--name",
            "development-test-split",
            "--split",
            "test",
            "--role",
            "development",
            "--reason",
            "local iterative benchmark",
        ],
    );
    let development_cohort_id = string_at(&development_cohort, "/cohort/id");
    let contamination = run_json(
        &database_url,
        ["contamination", "check", "--cohort", &development_cohort_id],
    );
    assert_eq!(contamination["status"], "clean");
    let benchmark_definition_path = directory.path().join("development-benchmark.json");
    std::fs::write(
        &benchmark_definition_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "contamination_report_id": contamination["id"],
            "name": "development gate",
            "kind": "development",
            "task": "Classify intentionally ambiguous support messages.",
            "labels": ["billing", "fraud"],
            "required_model_formats": [development_evaluation["run"]["source_identity"]["checkpoint_model_format"]],
            "cohorts": [{
                "cohort_id": development_cohort_id,
                "protocol": development_evaluation["run"]["protocol"],
                "disclosure": "row_content",
                "adaptation_eligible": true
            }],
            "contract": {
                "metric_requirements": [{
                    "target": {"kind": "overall"},
                    "metric": "accuracy",
                    "minimum": 1.0,
                    "minimum_support": 1
                }],
                "regression": {
                    "max_accuracy_drop": 0.02,
                    "max_macro_f1_drop": 0.02,
                    "minimum_accuracy_delta_lower_bound": null,
                    "minimum_macro_f1_delta_lower_bound": null,
                    "require_mcnemar_significance": false
                }
            }
        }))
        .expect("benchmark definition JSON"),
    )
    .expect("write benchmark definition");
    let benchmark = run_json(
        &database_url,
        [
            "benchmark",
            "create",
            "--definition",
            path(&benchmark_definition_path),
        ],
    );
    let benchmark_id = string_at(&benchmark, "/id");
    assert_eq!(
        run_json(&database_url, ["benchmark", "validate", &benchmark_id])["valid"],
        true
    );
    let run_mapping = format!("{development_cohort_id}={development_evaluation_id}");
    let acceptance = run_json(
        &database_url,
        ["benchmark", "assess", &benchmark_id, "--run", &run_mapping],
    );
    assert_eq!(acceptance["state"], "fail");
    let repeated_acceptance = run_json(
        &database_url,
        ["benchmark", "assess", &benchmark_id, "--run", &run_mapping],
    );
    assert_eq!(repeated_acceptance["id"], acceptance["id"]);
    let sealed_csv_path = directory.path().join("sealed.csv");
    std::fs::write(&sealed_csv_path, sealed_csv()).expect("write sealed CSV");
    let sealed_dataset = run_json(
        &database_url,
        [
            "dataset",
            "create",
            "--name",
            "sealed-support",
            "--task",
            "Classify intentionally ambiguous support messages.",
            "--label",
            "billing",
            "--label",
            "fraud",
        ],
    );
    let sealed_dataset_id = string_at(&sealed_dataset, "/id");
    let sealed_import = run_json(
        &database_url,
        [
            "dataset",
            "import",
            &sealed_dataset_id,
            "--input",
            path(&sealed_csv_path),
            "--format",
            "csv",
        ],
    );
    assert_eq!(sealed_import["accepted_rows"], 4);
    let sealed_snapshot = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &sealed_dataset_id,
            "--name",
            "sealed-acceptance-only",
            "--train-ratio",
            "0",
            "--validation-ratio",
            "0",
            "--test-ratio",
            "1",
            "--seed",
            "17",
        ],
    );
    let sealed_snapshot_id = string_at(&sealed_snapshot, "/snapshot/id");
    let sealed_cohort = run_json(
        &database_url,
        [
            "cohort",
            "create",
            &sealed_snapshot_id,
            "--name",
            "sealed-acceptance-test-split",
            "--split",
            "test",
            "--role",
            "sealed-acceptance",
            "--reason",
            "explicit final local acceptance gate",
        ],
    );
    let sealed_cohort_id = string_at(&sealed_cohort, "/cohort/id");
    let sealed_contamination = run_json(
        &database_url,
        ["contamination", "check", "--cohort", &sealed_cohort_id],
    );
    let sealed_benchmark_path = directory.path().join("sealed-benchmark.json");
    std::fs::write(
        &sealed_benchmark_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "contamination_report_id": sealed_contamination["id"],
            "name": "sealed acceptance gate",
            "kind": "sealed_acceptance",
            "task": "Classify intentionally ambiguous support messages.",
            "labels": ["billing", "fraud"],
            "required_model_formats": [evaluation["run"]["source_identity"]["checkpoint_model_format"]],
            "cohorts": [{
                "cohort_id": sealed_cohort_id,
                "protocol": evaluation["run"]["protocol"],
                "disclosure": "aggregate",
                "adaptation_eligible": false
            }],
            "contract": {
                "metric_requirements": [{
                    "target": {"kind": "overall"},
                    "metric": "accuracy",
                    "minimum": 0.01,
                    "minimum_support": 1
                }]
            }
        }))
        .expect("sealed benchmark definition JSON"),
    )
    .expect("write sealed benchmark definition");
    let sealed_benchmark = run_json(
        &database_url,
        [
            "benchmark",
            "create",
            "--definition",
            path(&sealed_benchmark_path),
        ],
    );
    let sealed_benchmark_id = string_at(&sealed_benchmark, "/id");

    let workflow_definition_path = directory.path().join("workflow-definition.json");
    std::fs::write(
        &workflow_definition_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "name": "bounded encoder loop",
            "dataset_id": dataset_id,
            "project_configuration_id": initialized["project_configuration"]["id"],
            "project_configuration_fingerprint": initialized["project_configuration"]["fingerprint"],
            "development_suite_id": benchmark_id,
            "development_suite_fingerprint": benchmark["fingerprint"],
            "sealed_suite_id": sealed_benchmark_id,
            "sealed_suite_fingerprint": sealed_benchmark["fingerprint"],
            "initial_allocation": {
                "total_rows": 20,
                "reserved_rows": 4,
                "policy": {"kind": "balanced"},
                "constraints": []
            },
            "governance": {"mode": "review_each_iteration"},
            "budget": {
                "maximum_iterations": 2,
                "maximum_initial_rows": 20,
                "maximum_cumulative_rows": 30,
                "maximum_generation_attempts": 100,
                "maximum_generation_requests": 50,
                "maximum_advisor_calls": 2,
                "maximum_advisor_tokens": 1000,
                "maximum_stage_attempts": 3
            },
            "advisor": {
                "backend": "fake",
                "model": "deterministic-v1",
                "base_url": null,
                "api_key_env": "SYNTH_ADVISOR_API_KEY",
                "egress_policy": "aggregate_only",
                "maximum_findings": 10,
                "maximum_representative_errors": 0,
                "maximum_actions": 5,
                "maximum_output_tokens": 1000,
                "temperature": 0.0
            },
            "policy": {
                "minimum_improvement": 0.01,
                "maximum_tolerated_regression": 0.02,
                "stop_on_inconclusive": true,
                "stop_on_invalid": true,
                "enable_advisor": true,
                "require_fresh_development_cohort_after_iterations": 2
            }
        }))
        .expect("workflow definition JSON"),
    )
    .expect("write workflow definition");
    let workflow_definition = run_json(
        &database_url,
        [
            "workflow",
            "define",
            "--definition",
            path(&workflow_definition_path),
        ],
    );
    assert_eq!(
        workflow_definition["benchmark_bundle"]["development_suite_id"],
        benchmark_id
    );
    assert_eq!(
        workflow_definition["benchmark_bundle"]["sealed_suite_id"],
        sealed_benchmark_id
    );
    let benchmark_bundle_binding = workflow_definition["benchmark_bundle"].clone();
    let workflow_definition_id = string_at(&workflow_definition, "/id");
    let workflow = run_json(
        &database_url,
        [
            "workflow",
            "start",
            &workflow_definition_id,
            "--initialize-only",
        ],
    );
    let workflow_id = string_at(&workflow, "/run/id");
    assert_eq!(workflow["attempt"]["stage"], "initial_allocation");
    assert_eq!(
        run_json(&database_url, ["workflow", "status", &workflow_id])["attempt_count"],
        1
    );
    let interrupted_workflows = run_json(&database_url, ["recovery", "list"]);
    assert!(
        interrupted_workflows
            .as_array()
            .expect("recovery records")
            .iter()
            .any(|record| {
                record["workflow_kind"] == "encoder_workflow"
                    && record["workflow_id"] == workflow_id
            })
    );
    assert_eq!(
        run_json(&database_url, ["workflow", "cancel", &workflow_id])["cancel_requested"],
        true
    );
    run_json(
        &database_url,
        ["recovery", "dismiss", "encoder-workflow", &workflow_id],
    );
    let automatic_workflow = run_json(
        &database_url,
        ["workflow", "start", &workflow_definition_id],
    );
    assert_eq!(automatic_workflow["run"]["current_stage"], "approval");
    assert_eq!(automatic_workflow["run"]["state"], "awaiting_approval");
    assert_eq!(automatic_workflow["attempt_count"], 20);
    assert_eq!(
        automatic_workflow["latest_attempt"]["state"],
        "awaiting_approval"
    );
    let workflow_artifact_kinds = automatic_workflow["attempts"]
        .as_array()
        .expect("workflow attempts")
        .iter()
        .flat_map(|attempt| {
            attempt["artifacts"]
                .as_array()
                .expect("attempt artifacts")
                .iter()
        })
        .map(|artifact| string_at(artifact, "/kind"))
        .collect::<Vec<_>>();
    for kind in [
        "initial_allocation",
        "generation_plan",
        "generation_job",
        "snapshot",
        "training_run",
        "checkpoint",
        "evaluation_run",
        "acceptance_assessment",
        "analysis_report",
        "advisory_assessment",
        "optimization_proposal",
    ] {
        assert!(workflow_artifact_kinds.contains(&kind.to_owned()));
    }
    let automatic_workflow_id = string_at(&automatic_workflow, "/run/id");
    let advisory_assessments = run_json(
        &database_url,
        [
            "advisor",
            "list",
            "--workflow-run-id",
            &automatic_workflow_id,
        ],
    );
    assert_eq!(
        advisory_assessments.as_array().expect("advisor list").len(),
        1
    );
    assert_eq!(
        advisory_assessments[0]["request"]["egress_policy"],
        "aggregate_only"
    );
    let completed_workflow = run_json(
        &database_url,
        ["workflow", "approve", &automatic_workflow_id],
    );
    assert_eq!(
        completed_workflow["run"]["state"], "development_complete",
        "workflow did not complete: {completed_workflow}"
    );
    assert_eq!(completed_workflow["run"]["iteration"], 1);
    let completed_artifact_kinds = completed_workflow["attempts"]
        .as_array()
        .expect("completed workflow attempts")
        .iter()
        .flat_map(|attempt| {
            attempt["artifacts"]
                .as_array()
                .expect("completed attempt artifacts")
                .iter()
        })
        .map(|artifact| string_at(artifact, "/kind"))
        .collect::<Vec<_>>();
    for kind in [
        "workflow_approval",
        "proposal_application",
        "iteration_generation_plan",
        "dataset_diff_generation_job",
        "iteration_snapshot",
        "iteration_training_run",
        "iteration_checkpoint",
        "iteration_evaluation_run",
        "evaluation_comparison",
        "followup_analysis_report",
        "stop_decision",
    ] {
        assert!(completed_artifact_kinds.contains(&kind.to_owned()));
    }
    let stop_decision_link = completed_workflow["attempts"]
        .as_array()
        .expect("completed workflow attempts")
        .iter()
        .flat_map(|attempt| attempt["artifacts"].as_array().expect("attempt artifacts"))
        .find(|artifact| artifact["kind"] == "stop_decision")
        .expect("stop decision link");
    let stop_decision_id = string_at(stop_decision_link, "/artifact_id");
    let stop_decision = run_json(&database_url, ["workflow", "stop-show", &stop_decision_id]);
    let stop_comparison_ids = stop_decision["comparison_ids"]
        .as_array()
        .expect("stop comparison IDs");
    assert_eq!(stop_comparison_ids.len(), 1);
    let assessment_id = string_at(&stop_decision, "/acceptance_assessment_id");
    let stop_assessment = run_json(
        &database_url,
        ["benchmark", "assessment-show", &assessment_id],
    );
    let assessment_comparison_ids = stop_assessment["comparison_ids"]
        .as_object()
        .expect("assessment comparison IDs");
    assert_eq!(assessment_comparison_ids.len(), 1);
    assert_eq!(
        assessment_comparison_ids
            .get(&development_cohort_id)
            .and_then(serde_json::Value::as_str),
        stop_comparison_ids[0].as_str(),
        "the decision-grade assessment and stop decision must pin the same paired comparison"
    );
    let finalized_workflow = run_json(
        &database_url,
        ["workflow", "finalize", &automatic_workflow_id],
    );
    assert_eq!(
        finalized_workflow["run"]["current_stage"],
        "sealed_evaluation"
    );
    assert_eq!(finalized_workflow["latest_attempt"]["state"], "completed");
    let promoted_workflow = run_json(
        &database_url,
        ["workflow", "promote", &automatic_workflow_id],
    );
    assert_eq!(promoted_workflow["run"]["state"], "completed");
    let promotion_link = promoted_workflow["attempts"]
        .as_array()
        .expect("promotion attempts")
        .iter()
        .flat_map(|attempt| {
            attempt["artifacts"]
                .as_array()
                .expect("promotion artifacts")
        })
        .find(|artifact| artifact["kind"] == "model_promotion")
        .expect("model promotion link");
    let promotion_id = string_at(promotion_link, "/artifact_id");
    let promotion = run_json(&database_url, ["workflow", "promotion-show", &promotion_id]);
    assert_eq!(promotion["state"], "promoted");
    assert!(promotion["training_benchmark_check_id"].is_string());
    assert!(promotion["training_benchmark_check_fingerprint"].is_string());
    let promotion_trace = run_json(
        &database_url,
        ["provenance", "model-promotion", &promotion_id],
    );
    let promotion_trace_text = serde_json::to_string(&promotion_trace).expect("promotion trace");
    for kind in [
        "model_promotion",
        "checkpoint",
        "snapshot",
        "training_benchmark_check",
        "benchmark_bundle",
        "contamination_report",
        "acceptance_assessment",
        "evaluation_run",
    ] {
        assert!(promotion_trace_text.contains(kind));
    }
    let workflow_trace = run_json(
        &database_url,
        ["provenance", "workflow-run", &automatic_workflow_id],
    );
    assert_eq!(workflow_trace["kind"], "workflow_run");
    let development_exposures =
        run_json(&database_url, ["exposure", "list", &development_cohort_id]);
    let development_purposes = development_exposures
        .as_array()
        .expect("development exposures")
        .iter()
        .map(|value| string_at(value, "/purpose"))
        .collect::<std::collections::BTreeSet<_>>();
    for purpose in [
        "development_evaluation",
        "diagnosis",
        "advisor",
        "optimization",
        "comparison",
    ] {
        assert!(development_purposes.contains(purpose));
    }
    let sealed_exposures = run_json(&database_url, ["exposure", "list", &sealed_cohort_id]);
    assert_eq!(
        sealed_exposures.as_array().expect("sealed exposures").len(),
        1
    );
    assert_eq!(sealed_exposures[0]["purpose"], "acceptance");
    assert_eq!(sealed_exposures[0]["disclosure"], "aggregate");
    assert_eq!(sealed_exposures[0]["adaptation_eligible"], false);
    assert!(
        promoted_workflow["evidence_risk"]
            .as_array()
            .expect("workflow evidence risk")
            .iter()
            .any(|value| value["cohort_id"] == sealed_cohort_id)
    );
    let preauthorized_definition_path = directory.path().join("preauthorized-workflow.json");
    let preauthorized_request = serde_json::json!({
        "name": "preauthorized bounded encoder loop",
        "dataset_id": dataset_id,
        "project_configuration_id": initialized["project_configuration"]["id"],
        "project_configuration_fingerprint": initialized["project_configuration"]["fingerprint"],
        "development_suite_id": benchmark_id,
        "development_suite_fingerprint": benchmark["fingerprint"],
        "sealed_suite_id": sealed_benchmark_id,
        "sealed_suite_fingerprint": sealed_benchmark["fingerprint"],
        "initial_allocation": {
            "total_rows": 24,
            "reserved_rows": 4,
            "policy": {"kind": "balanced"},
            "constraints": []
        },
        "governance": {
            "mode": "preauthorized_bounded",
            "envelope": {
                "maximum_iterations": 1,
                "maximum_additional_rows": 4,
                "maximum_generation_requests": 50,
                "maximum_advisor_calls": 0,
                "maximum_advisor_tokens": 0,
                "permitted_generation_backend": "fake",
                "permitted_generation_model": "deterministic-v1",
                "permitted_training_backend": "hashing-linear",
                "permitted_training_configuration_fingerprints": []
            }
        },
        "budget": {
            "maximum_iterations": 1,
            "maximum_initial_rows": 24,
            "maximum_cumulative_rows": 28,
            "maximum_generation_attempts": 100,
            "maximum_generation_requests": 50,
            "maximum_advisor_calls": 0,
            "maximum_advisor_tokens": 0,
            "maximum_stage_attempts": 3
        },
        "policy": {
            "minimum_improvement": 0.01,
            "maximum_tolerated_regression": 0.02,
            "stop_on_inconclusive": true,
            "stop_on_invalid": true,
            "enable_advisor": false,
            "require_fresh_development_cohort_after_iterations": 2
        }
    });
    let mismatched_definition_path = directory.path().join("mismatched-workflow.json");
    let mut mismatched_request = preauthorized_request.clone();
    let mut mismatched_binding = benchmark_bundle_binding.clone();
    mismatched_binding["bundle_id"] = serde_json::json!(uuid::Uuid::new_v4());
    mismatched_request["benchmark_bundle"] = mismatched_binding;
    std::fs::write(
        &mismatched_definition_path,
        serde_json::to_vec_pretty(&mismatched_request)
            .expect("mismatched workflow definition JSON"),
    )
    .expect("write mismatched workflow definition");
    let mismatched = run(
        &database_url,
        [
            "workflow",
            "define",
            "--definition",
            path(&mismatched_definition_path),
        ],
    );
    assert!(!mismatched.status.success());
    assert!(
        String::from_utf8_lossy(&mismatched.stderr)
            .contains("does not match the binding derived from persisted benchmark evidence")
    );

    std::fs::write(
        &preauthorized_definition_path,
        serde_json::to_vec_pretty(&preauthorized_request)
            .expect("preauthorized workflow definition JSON"),
    )
    .expect("write preauthorized workflow definition");
    let preauthorized_definition = run_json(
        &database_url,
        [
            "workflow",
            "define",
            "--definition",
            path(&preauthorized_definition_path),
        ],
    );
    assert_eq!(
        preauthorized_definition["benchmark_bundle"],
        benchmark_bundle_binding
    );
    let preauthorized_definition_id = string_at(&preauthorized_definition, "/id");
    let preauthorized_workflow = run_json(
        &database_url,
        ["workflow", "start", &preauthorized_definition_id],
    );
    assert_eq!(
        preauthorized_workflow["run"]["state"],
        "development_complete"
    );
    assert!(
        preauthorized_workflow["attempts"]
            .as_array()
            .expect("preauthorized attempts")
            .iter()
            .flat_map(|attempt| attempt["artifacts"].as_array().expect("artifacts"))
            .any(|artifact| artifact["kind"] == "workflow_approval")
    );

    let analysis = run_json(
        &database_url,
        [
            "analysis",
            "create",
            &evaluation_id,
            "--minimum-support",
            "1",
            "--comparison-id",
            &comparison_id,
        ],
    );
    let analysis_id = string_at(&analysis, "/id");
    assert!(analysis["error_count"].as_u64().expect("error count") > 0);
    assert_eq!(
        string_at(&analysis, "/comparison_diagnosis/comparison_id"),
        comparison_id
    );

    let findings = run_json(
        &database_url,
        ["analysis", "findings", &analysis_id, "--limit", "10"],
    );
    let finding_key = findings
        .as_array()
        .and_then(|values| values.first())
        .and_then(|finding| finding["key"].as_str())
        .expect("ranked finding key")
        .to_owned();
    let finding = run_json(
        &database_url,
        ["analysis", "finding", &analysis_id, &finding_key],
    );
    assert_eq!(finding["rank"], 1);
    let reviewed = run_json(
        &database_url,
        [
            "analysis",
            "review",
            &analysis_id,
            &finding_key,
            "--state",
            "candidate-for-more-data",
            "--note",
            "offline workflow review",
        ],
    );
    assert_eq!(reviewed["state"], "candidate_for_more_data");
    let review_id = string_at(&reviewed, "/id");
    let review_trace = run_json(
        &database_url,
        ["provenance", "analysis-finding-review", &review_id],
    );
    let review_trace = serde_json::to_string(&review_trace).expect("review trace JSON");
    for kind in [
        "analysis_finding_review",
        "analysis_report",
        "evaluation_comparison",
        "evaluation_run",
    ] {
        assert!(
            review_trace.contains(kind),
            "review trace is missing {kind}"
        );
    }
    let weak_cells = run_json(
        &database_url,
        ["analysis", "weak-cells", &analysis_id, "--limit", "10"],
    );
    assert!(!weak_cells.as_array().expect("weak cells").is_empty());
    let persistent = run_json(
        &database_url,
        ["analysis", "comparison-group", &analysis_id, "persistent"],
    );
    assert!(
        !persistent
            .as_array()
            .expect("persistent comparison evidence")
            .is_empty()
    );
    let evidence_path = directory.path().join("finding-evidence.jsonl");
    let evidence_export = run_json(
        &database_url,
        [
            "analysis",
            "evidence",
            &analysis_id,
            &finding_key,
            "--format",
            "jsonl",
            "--file",
            path(&evidence_path),
        ],
    );
    assert!(evidence_export["rows"].as_u64().expect("evidence rows") > 0);
    assert!(
        std::fs::read_to_string(&evidence_path)
            .expect("read evidence export")
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok())
    );
    let evidence_csv_path = directory.path().join("finding-evidence.csv");
    let csv_export = run_json(
        &database_url,
        [
            "analysis",
            "evidence",
            &analysis_id,
            &finding_key,
            "--format",
            "csv",
            "--file",
            path(&evidence_csv_path),
        ],
    );
    assert_eq!(csv_export["rows"], evidence_export["rows"]);
    assert!(
        std::fs::read_to_string(&evidence_csv_path)
            .expect("read evidence CSV")
            .starts_with("category,prediction_id")
    );

    let training_protocol_path = directory.path().join("training-optimization.json");
    let mut training_protocol = OptimizationProtocol::legacy(2, 1);
    training_protocol.recommendation_kinds = vec![
        RecommendationKind::DataGeneration,
        RecommendationKind::TrainingConfiguration,
    ];
    training_protocol.training_candidates = Some(TrainingCandidateRequest {
        maximum_candidates: 1,
    });
    std::fs::write(
        &training_protocol_path,
        serde_json::to_vec_pretty(&training_protocol.normalize().expect("training protocol"))
            .expect("serialize training protocol"),
    )
    .expect("write training protocol");
    let training_proposal = run_json(
        &database_url,
        [
            "optimize",
            "propose",
            &analysis_id,
            "--protocol",
            path(&training_protocol_path),
            "--training-space",
            path(&training_space_path),
        ],
    );
    let training_proposal_id = string_at(&training_proposal, "/id");
    assert_eq!(
        training_proposal["training_candidate_set"]["candidates"]
            .as_array()
            .expect("training candidates")
            .len(),
        1
    );
    let training_candidates_export = directory.path().join("training-candidates.csv");
    run_json(
        &database_url,
        [
            "optimize",
            "training-candidates",
            &training_proposal_id,
            "--format",
            "csv",
            "--file",
            path(&training_candidates_export),
        ],
    );
    assert!(
        std::fs::read_to_string(&training_candidates_export)
            .expect("training candidate export")
            .starts_with("proposal_id,candidate_id")
    );

    let proposal = run_json(
        &database_url,
        [
            "optimize",
            "propose",
            &analysis_id,
            "--budget",
            "2",
            "--minimum-support",
            "1",
        ],
    );
    let proposal_id = string_at(&proposal, "/id");
    assert_eq!(proposal["additional_example_budget"], 2);

    let application = run_json(&database_url, ["optimize", "legacy-apply", &proposal_id]);
    let applied_plan_id = string_at(&application, "/generation_plan/id");
    assert_eq!(application["already_applied"], false);
    let repeated = run_json(&database_url, ["optimize", "legacy-apply", &proposal_id]);
    assert_eq!(repeated["already_applied"], true);
    assert_eq!(string_at(&repeated, "/generation_plan/id"), applied_plan_id);

    let preview = run_json(
        &database_url,
        [
            "optimize",
            "preview",
            &analysis_id,
            "--protocol",
            path(&optimization_protocol_path),
        ],
    );
    assert_eq!(preview["persisted"], false);
    let decision_proposal = run_json(
        &database_url,
        [
            "optimize",
            "propose",
            &analysis_id,
            "--protocol",
            path(&optimization_protocol_path),
        ],
    );
    let decision_proposal_id = string_at(&decision_proposal, "/id");
    let review = run_json(
        &database_url,
        [
            "optimize",
            "review",
            &decision_proposal_id,
            "--state",
            "approved-for-plan-creation",
        ],
    );
    let review_id = string_at(&review, "/id");
    let decision_application = run_json(
        &database_url,
        [
            "optimize",
            "apply",
            &decision_proposal_id,
            "--approval-id",
            &review_id,
        ],
    );
    assert_eq!(decision_application["generation_started"], false);
    let campaign = run_json(
        &database_url,
        [
            "campaign",
            "create",
            &decision_proposal_id,
            "--approval-id",
            &review_id,
        ],
    );
    let campaign_id = string_at(&campaign, "/id");
    let decision_plan_id = string_at(&decision_application, "/generation_plan/id");
    let campaign_link = run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--generation-plan-id",
            &decision_plan_id,
        ],
    );
    assert_eq!(campaign_link["already_linked"], false);

    let optimized_generation = run_json(
        &database_url,
        [
            "generate",
            &decision_plan_id,
            "--config",
            path(&config_path),
            "--batch-size",
            "1",
        ],
    );
    assert_eq!(optimized_generation["state"], "completed");
    let optimized_job_id = string_at(&optimized_generation, "/id");
    run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--generation-job-id",
            &optimized_job_id,
        ],
    );

    let candidate_snapshot = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &dataset_id,
            "--config",
            path(&config_path),
        ],
    );
    let candidate_snapshot_id = string_at(&candidate_snapshot, "/snapshot/id");
    run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--snapshot-id",
            &candidate_snapshot_id,
        ],
    );

    let candidate_training = run_json(
        &database_url,
        [
            "training",
            "run",
            &candidate_snapshot_id,
            "--config",
            path(&config_path),
        ],
    );
    let candidate_training_id = string_at(&candidate_training, "/run/id");
    let candidate_checkpoint_id = candidate_training["checkpoints"]
        .as_array()
        .expect("candidate checkpoints")
        .iter()
        .find(|checkpoint| checkpoint["is_final"] == true)
        .and_then(|checkpoint| checkpoint["id"].as_str())
        .expect("candidate final checkpoint")
        .to_owned();
    run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--training-run-id",
            &candidate_training_id,
        ],
    );
    run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--checkpoint-id",
            &candidate_checkpoint_id,
        ],
    );

    let candidate_evaluation = run_json(
        &database_url,
        [
            "evaluation",
            "run",
            &candidate_checkpoint_id,
            "--snapshot-id",
            &snapshot_id,
            "--config",
            path(&config_path),
        ],
    );
    let candidate_evaluation_id = string_at(&candidate_evaluation, "/run/id");
    run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--evaluation-run-id",
            &candidate_evaluation_id,
        ],
    );
    let outcome_comparison = run_json(
        &database_url,
        [
            "evaluation",
            "compare",
            &evaluation_id,
            &candidate_evaluation_id,
        ],
    );
    let outcome_comparison_id = string_at(&outcome_comparison, "/id");
    run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--comparison-id",
            &outcome_comparison_id,
        ],
    );
    let follow_up_analysis = run_json(
        &database_url,
        [
            "analysis",
            "create",
            &candidate_evaluation_id,
            "--minimum-support",
            "1",
            "--comparison-id",
            &outcome_comparison_id,
        ],
    );
    let follow_up_analysis_id = string_at(&follow_up_analysis, "/id");
    run_json(
        &database_url,
        [
            "campaign",
            "link",
            &campaign_id,
            "--analysis-report-id",
            &follow_up_analysis_id,
        ],
    );
    let outcome = run_json(
        &database_url,
        [
            "campaign",
            "assess",
            &campaign_id,
            "--comparison-id",
            &outcome_comparison_id,
            "--allow-non-significant",
        ],
    );
    assert_eq!(outcome["already_assessed"], false);
    let outcome_id = string_at(&outcome, "/outcome/id");

    let campaign_show = run_json(&database_url, ["campaign", "show", &campaign_id]);
    assert_eq!(campaign_show["links"].as_array().expect("links").len(), 8);
    assert!(campaign_show["outcome"].is_object());
    let campaign_trace = run_json(
        &database_url,
        ["provenance", "optimization-campaign", &campaign_id],
    );
    let campaign_trace = serde_json::to_string(&campaign_trace).expect("campaign trace");
    assert!(campaign_trace.contains("optimization_campaign"));
    assert!(campaign_trace.contains("optimization_proposal_review"));
    let outcome_trace = run_json(
        &database_url,
        ["provenance", "optimization-outcome", &outcome_id],
    );
    assert!(
        serde_json::to_string(&outcome_trace)
            .expect("outcome trace")
            .contains("evaluation_comparison")
    );

    let scenarios = run_json(
        &database_url,
        [
            "optimize",
            "scenarios",
            &analysis_id,
            "--protocol",
            path(&optimization_protocol_path),
        ],
    );
    assert_eq!(
        scenarios["scenarios"].as_array().expect("scenarios").len(),
        4
    );
    let scenario_group_id = string_at(&scenarios, "/id");
    let scenario_id = string_at(&scenarios, "/scenarios/0/id");
    let shown_scenarios = run_json(
        &database_url,
        ["optimize", "scenario-show", &scenario_group_id],
    );
    assert_eq!(shown_scenarios["fingerprint"], scenarios["fingerprint"]);
    let materialized = run_json(
        &database_url,
        [
            "optimize",
            "scenario-materialize",
            &scenario_group_id,
            &scenario_id,
        ],
    );
    assert_eq!(materialized["already_materialized"], false);

    let recommendation_export = directory.path().join("recommendations.jsonl");
    let summary_export = directory.path().join("proposal-summary.csv");
    run_json(
        &database_url,
        [
            "optimize",
            "export-recommendations",
            &decision_proposal_id,
            "--file",
            path(&recommendation_export),
        ],
    );
    run_json(
        &database_url,
        [
            "optimize",
            "export-summary",
            &decision_proposal_id,
            "--format",
            "csv",
            "--file",
            path(&summary_export),
        ],
    );
    assert!(
        std::fs::read_to_string(&recommendation_export)
            .expect("recommendation export")
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok())
    );
    assert!(
        std::fs::read_to_string(&summary_export)
            .expect("summary export")
            .starts_with("proposal_id,analysis_report_id")
    );

    let trace = run_json(
        &database_url,
        ["provenance", "optimization-proposal", &proposal_id],
    );
    let trace_text = serde_json::to_string(&trace).expect("trace JSON");
    for kind in [
        "optimization_proposal",
        "analysis_report",
        "evaluation_run",
        "checkpoint",
        "training_run",
        "snapshot",
        "dataset",
        "project_configuration",
        "dataset_import",
        "generation_job",
    ] {
        assert!(trace_text.contains(kind), "trace is missing {kind}");
    }

    let export_path = directory.path().join("accepted.jsonl");
    let exported = run_json(
        &database_url,
        [
            "export",
            &dataset_id,
            "--format",
            "jsonl",
            "--file",
            path(&export_path),
        ],
    );
    // The approved optimization evidence comes from the independent
    // development dataset, while its four-row diff is translated onto this
    // workflow's training dataset. Export therefore contains the original 22
    // accepted rows plus the correctly targeted diff.
    assert_eq!(exported["row_count"], 26);
    assert_eq!(
        std::fs::read_to_string(&export_path)
            .expect("read export")
            .lines()
            .count(),
        26
    );

    let listed = run_json(
        &database_url,
        [
            "job",
            "list",
            "--dataset-id",
            &dataset_id,
            "--state",
            "completed",
            "--limit",
            "1",
            "--summary",
        ],
    );
    assert_eq!(listed["page"]["returned"], 1);

    let human = run_text(
        &database_url,
        ["dataset", "list", "--name", "ambiguous", "--limit", "1"],
    );
    assert!(human.contains("id:"));
    assert!(!human.trim_start().starts_with('{'));

    let final_doctor = run_json(&database_url, ["doctor", "--config", path(&config_path)]);
    assert_eq!(final_doctor["healthy"], true);

    let promotion_uuid = uuid::Uuid::parse_str(&promotion_id).expect("promotion UUID");
    tokio::runtime::Runtime::new()
        .expect("tamper runtime")
        .block_on(async {
            let pool = sqlx::SqlitePool::connect(&database_url)
                .await
                .expect("tamper database connects");
            let artifact_json: String =
                sqlx::query_scalar("SELECT artifact_json FROM model_promotions WHERE id = ?")
                    .bind(promotion_uuid)
                    .fetch_one(&pool)
                    .await
                    .expect("promotion JSON");
            let mut artifact: ModelPromotion =
                serde_json::from_str(&artifact_json).expect("promotion decodes");
            artifact.checkpoint_id = uuid::Uuid::new_v4();
            artifact.checkpoint_fingerprint = "sha256:substituted-checkpoint".into();
            artifact.fingerprint = artifact
                .reproduce_fingerprint()
                .expect("tampered promotion fingerprint");
            sqlx::query("UPDATE model_promotions SET artifact_json = ? WHERE id = ?")
                .bind(serde_json::to_string(&artifact).expect("promotion encodes"))
                .bind(promotion_uuid)
                .execute(&pool)
                .await
                .expect("artifact-only tampering");
        });
    assert!(
        !run(&database_url, ["workflow", "promotion-show", &promotion_id],)
            .status
            .success(),
        "artifact-only checkpoint substitution must fail against normalized promotion facts"
    );
}

fn string_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {value}"))
        .to_owned()
}

fn path(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
}

fn config(artifacts: &Path) -> String {
    format!(
        r#"version = 1

[dataset]
name = "ambiguous-support"
task = "Classify intentionally ambiguous support messages."
labels = ["billing", "fraud"]

[generation]
target_per_cell = 7
batch_size = 1
max_retries = 0
max_attempt_multiplier = 2
backend = "fake"
model = "deterministic-v1"
seed = 7

[snapshot]
name = "ambiguous-baseline"
train_ratio = 0.5
validation_ratio = 0.0
test_ratio = 0.5
seed = 7

[training]
backend = "hashing-linear"
feature_dimension = 16
epochs = 1
learning_rate = 0.1
l2 = 0.0
checkpoint_every = 1
seed = 7
artifact_root = "{}"

[evaluation]
split = "test"
"#,
        artifacts.to_string_lossy().replace('\\', "/")
    )
}

fn ambiguous_csv() -> String {
    let billing = ["!", "!!", "!!!", ".!", "!?", "!?!"];
    let fraud = ["?", "??", "???", ".?", "?!", "?!?"];
    let mut csv = String::from("text,label\n");
    for punctuation in billing {
        csv.push_str(&format!("same support message{punctuation},billing\n"));
    }
    for punctuation in fraud {
        csv.push_str(&format!("same support message{punctuation},fraud\n"));
    }
    csv
}

fn development_csv() -> String {
    let billing = ["!", "!!", "!!!", ".!", "!?", "!?!"];
    let fraud = ["?", "??", "???", ".?", "?!", "?!?"];
    let mut csv = String::from("text,label\n");
    for punctuation in billing {
        csv.push_str(&format!(
            "independent benchmark message{punctuation},billing\n"
        ));
    }
    for punctuation in fraud {
        csv.push_str(&format!(
            "independent benchmark message{punctuation},fraud\n"
        ));
    }
    csv
}

fn sealed_csv() -> &'static str {
    "text,label\n\
sealed invoice reconciliation request,billing\n\
sealed subscription renewal question,billing\n\
sealed card takeover warning,fraud\n\
sealed identity theft notification,fraud\n"
}
