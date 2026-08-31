#[allow(dead_code)]
mod support;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use support::{run, run_json};

#[test]
fn offline_supervisor_pauses_repairs_canaries_completes_and_traces() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sidecar = root.join("adapters/research-agent-pi/dist/main.js");
    if !sidecar.is_file() {
        eprintln!(
            "skipping process-level supervisor acceptance because {} is not built",
            sidecar.display()
        );
        return;
    }
    let temporary = tempfile::tempdir().expect("temporary directory");
    let database = temporary.path().join("supervisor-cli.db");
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        database.to_string_lossy().replace('\\', "/")
    );
    let dataset = run_json(
        &database_url,
        [
            "dataset",
            "create",
            "--name",
            "supervised-support",
            "--task",
            "Classify support requests",
            "--label",
            "billing",
        ],
    );
    let plan = run_json(
        &database_url,
        ["plan", "create", text(&dataset, "id"), "--per-cell", "1"],
    );
    run_json(
        &database_url,
        [
            "backend",
            "configure",
            "--base-url",
            "https://provider.example/v1",
            "--model",
            "independent-critic",
        ],
    );
    let external_contract_file = temporary.path().join("external-contract.json");
    let mut external_contract = contract_document(text(&plan, "id"));
    external_contract["evaluator"] = json!("openai_compatible");
    std::fs::write(
        &external_contract_file,
        serde_json::to_vec_pretty(&external_contract).unwrap(),
    )
    .unwrap();
    let external_preview = run_json(
        &database_url,
        [
            "supervisor",
            "contract-preview",
            &path(&external_contract_file),
        ],
    );
    assert_eq!(
        external_preview["evaluator"]["backend"],
        "openai-compatible"
    );
    assert_eq!(
        external_preview["quality_policy"]["egress_policy"],
        "external_candidate_text"
    );
    external_contract["budgets"]["maximum_cost_microunits"] = json!(1000);
    let unpriced_contract_file = temporary.path().join("unpriced-contract.json");
    std::fs::write(
        &unpriced_contract_file,
        serde_json::to_vec_pretty(&external_contract).unwrap(),
    )
    .unwrap();
    let unpriced = run(
        &database_url,
        [
            "supervisor",
            "contract-preview",
            &path(&unpriced_contract_file),
        ],
    );
    assert!(!unpriced.status.success());
    assert!(
        String::from_utf8_lossy(&unpriced.stderr)
            .contains("provider token pricing is explicitly pinned")
    );
    let contract_file = temporary.path().join("contract.json");
    std::fs::write(
        &contract_file,
        serde_json::to_vec_pretty(&contract_document(text(&plan, "id"))).unwrap(),
    )
    .unwrap();
    let contract_file = path(&contract_file);
    let preview = run_json(
        &database_url,
        ["supervisor", "contract-preview", &contract_file],
    );
    assert_eq!(preview["plan"]["id"], plan["id"]);
    let contract = run_json(
        &database_url,
        ["supervisor", "contract-create", &contract_file],
    );
    let contract_id = text(&contract, "id").to_owned();
    assert_eq!(contract["schema_version"], 2);
    let started = run_json(
        &database_url,
        [
            "supervisor",
            "start",
            &contract_id,
            "--guidance",
            "Use natural customer language.",
        ],
    );
    let run_id = text(&started, "run/id").to_owned();
    assert_eq!(started["state"], "queued");

    let paused = run_json(&database_url, ["supervisor", "run", &run_id]);
    if paused["status"]["state"] != "paused" {
        let job = run_json(
            &database_url,
            ["job", "status", text(&paused, "generation_job_id")],
        );
        let issues = run_json(&database_url, ["supervisor", "issues", &run_id]);
        panic!(
            "unexpected supervisor outcome: {paused:#}\ngeneration job: {job:#}\nevents: {issues:#}"
        );
    }
    assert_eq!(paused["decisions"][0]["state"], "pause_for_diagnosis");
    let issues = run_json(&database_url, ["supervisor", "issues", &run_id]);
    assert_eq!(issues["windows"].as_array().unwrap().len(), 1);
    assert!(
        issues["decisions"][0]["issues"]
            .as_array()
            .unwrap()
            .iter()
            .any(|issue| issue == "shortcut_risk")
    );

    let script_file = temporary.path().join("diagnosis.json");
    std::fs::write(
        &script_file,
        serde_json::to_vec_pretty(&diagnosis_script()).unwrap(),
    )
    .unwrap();
    let script_file = path(&script_file);
    let sidecar = path(&sidecar);
    let diagnosed = run_json(
        &database_url,
        [
            "supervisor",
            "diagnose",
            &run_id,
            "--script",
            &script_file,
            "--pi-sidecar",
            &sidecar,
        ],
    );
    assert_eq!(
        diagnosed["session"]["state"], "awaiting_review",
        "{diagnosed:#}"
    );
    let session_id = text(&diagnosed, "session/id").to_owned();
    let revision = run_json(&database_url, ["supervisor", "revision-show", &session_id]);
    assert_eq!(revision["proposal"]["revision_sequence"], 1);

    let reviewed = run_json(
        &database_url,
        [
            "supervisor",
            "revision-review",
            &run_id,
            &session_id,
            "--approve",
            "--reason",
            "Narrow guidance-only repair with a mandatory canary",
        ],
    );
    assert_eq!(reviewed["status"]["state"], "canary");
    assert_eq!(reviewed["candidate_prompt"]["sequence"], 1);

    let canary = run_json(&database_url, ["supervisor", "canary", &run_id]);
    assert_eq!(canary["status"]["state"], "running");
    assert_eq!(canary["decisions"][0]["state"], "revision_passed");
    assert_eq!(canary["status"]["active_prompt"]["sequence"], 1);
    let completed = run_json(&database_url, ["supervisor", "run", &run_id]);
    assert_eq!(completed["status"]["state"], "completed");

    let canary_job_id = text(&canary, "generation_job_id").to_owned();
    let rows = run_json(
        &database_url,
        ["rows", "--job-id", &canary_job_id, "--status", "accepted"],
    );
    let row_id = rows[0]["id"].as_str().unwrap();
    let trace = run_json(&database_url, ["supervisor", "trace-row", row_id]);
    assert_eq!(trace["prompt_version"]["sequence"], 1);
    assert_eq!(trace["decisions"][0]["state"], "revision_passed");
    let generic_prompt = run(
        &database_url,
        ["job", "prompt", &canary_job_id, "--cell-index", "0"],
    );
    assert!(!generic_prompt.status.success());
    assert!(
        String::from_utf8_lossy(&generic_prompt.stderr)
            .contains("row-specific immutable prompt schedules")
    );
    let integrity = run_json(&database_url, ["supervisor", "integrity"]);
    assert_eq!(integrity["errors"].as_array().unwrap().len(), 0);
    let doctor = run_json(&database_url, ["doctor"]);
    assert_eq!(doctor["healthy"], true);
    assert!(doctor["checks"].as_array().unwrap().iter().any(|check| {
        check["name"] == "generation_supervisor_facts" && check["status"] == "pass"
    }));
}

fn contract_document(plan_id: &str) -> Value {
    json!({
        "schema_version": 1,
        "plan_id": plan_id,
        "generator": "fake",
        "evaluator": "fake",
        "quality_policy": {
            "thresholds": {
                "minimum_assigned_label_score": 7000,
                "minimum_label_margin": 500,
                "minimum_dimension_adherence_score": 7000,
                "maximum_label_leakage_risk": 2000,
                "maximum_shortcut_risk": 2000,
                "minimum_evaluator_confidence": 7000,
                "borderline_margin": 500
            },
            "invalid_output_policy": "quarantine",
            "budgets": {
                "maximum_rows_per_batch": 1,
                "maximum_evaluator_requests": 1,
                "maximum_attempts_per_request": 1,
                "maximum_input_tokens": 10000,
                "maximum_output_tokens": 10000,
                "maximum_total_tokens": 20000
            }
        },
        "batch_thresholds": {
            "minimum_qualified_rate": 8000,
            "maximum_borderline_rate": 2000,
            "maximum_quarantined_rate": 2000,
            "maximum_invalid_rate": 1000,
            "maximum_exact_duplicate_rate": 0,
            "maximum_normalized_duplicate_rate": 0,
            "maximum_template_repetition_rate": 2000,
            "maximum_qualified_rate_drop": 1000,
            "maximum_shortcut_concentration": 2000,
            "required_patterns": []
        },
        "monitoring": {
            "initial_canary_rows_per_scope": 1,
            "revision_canary_rows_per_scope": 1,
            "rolling_window_rows_per_scope": 1,
            "minimum_evidence_rows_per_scope": 1,
            "baseline_minimum_rows_per_scope": 1,
            "scope": "cell",
            "baseline_policy": "initial_canary",
            "systemic_pause_minimum_scopes": 2
        },
        "budgets": {
            "maximum_generation_segments": 2,
            "maximum_generated_rows": 4,
            "maximum_quality_audits": 2,
            "maximum_evaluator_requests": 2,
            "maximum_evaluator_attempts": 2,
            "maximum_evaluator_input_tokens": 20000,
            "maximum_evaluator_output_tokens": 20000,
            "maximum_evaluator_total_tokens": 40000,
            "maximum_prompt_revisions": 1,
            "maximum_revision_canaries": 1,
            "maximum_pi_model_turns": 1,
            "maximum_pi_tool_calls": 7,
            "maximum_pi_input_tokens": 10000,
            "maximum_pi_output_tokens": 5000,
            "maximum_retries_per_external_call": 1,
            "maximum_duration_seconds": 600
        },
        "approval_policy": {"mode": "explicit_review"},
        "revision_policy": {
            "maximum_instructions": 2,
            "maximum_characters_per_instruction": 200,
            "maximum_total_characters": 300,
            "allowed_kind": "replace_generation_guidance",
            "protected_fields": [
                "system_prompt", "output_schema", "target_label", "target_dimensions",
                "construction_graph", "semantic_authority", "quality_thresholds",
                "budgets", "safety_instructions"
            ]
        }
    })
}

fn diagnosis_script() -> Value {
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

fn text<'a>(value: &'a Value, pointer: &str) -> &'a str {
    value
        .pointer(&format!("/{pointer}"))
        .and_then(Value::as_str)
        .unwrap()
}

fn path(value: &Path) -> String {
    value.to_string_lossy().into_owned()
}
