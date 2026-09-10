#[allow(dead_code)]
mod support;

use std::{
    collections::BTreeSet,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    thread,
    time::{Duration, Instant},
};

use serde_json::Value;
use support::{run, run_json};
use uuid::Uuid;

static EXTERNAL_QUALITY_TEST_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn external_quality_audit_runs_through_a_loopback_openai_compatible_process() {
    let _serial = EXTERNAL_QUALITY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback provider");
    let address = listener.local_addr().expect("loopback address");
    listener
        .set_nonblocking(true)
        .expect("nonblocking provider listener");

    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("loopback-quality-cli.db");
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
            "loopback-quality",
            "--task",
            "Classify support requests",
            "--label",
            "billing",
            "--label",
            "fraud",
        ],
    );
    let dataset_id = string_at(&dataset, "/id");
    let plan = run_json(
        &database_url,
        ["plan", "create", &dataset_id, "--per-cell", "1"],
    );
    let plan_id = string_at(&plan, "/id");
    run_json(&database_url, ["generate", &plan_id, "--backend", "fake"]);
    run_json(
        &database_url,
        [
            "backend",
            "configure",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "loopback-quality-model",
        ],
    );
    let audit = run_json(
        &database_url,
        [
            "quality",
            "audit-create",
            &dataset_id,
            "--preset",
            "fast",
            "--egress",
            "external-candidate-text",
            "--evaluator",
            "openai-compatible",
        ],
    );
    let run_id = string_at(&audit, "/audit_run/id");
    // Start the provider deadline only after the independent CLI setup. A busy
    // workstation must not exhaust the connection window while creating data.
    let provider = thread::spawn(move || serve_one_quality_response(listener));
    let outcome = run_json(
        &database_url,
        [
            "quality",
            "audit-start",
            &run_id,
            "--api-key-env",
            "_ENCODER_GYM_TEST_MISSING_QUALITY_API_KEY",
        ],
    );
    provider
        .join()
        .expect("loopback provider thread")
        .unwrap_or_else(|error| panic!("loopback provider response: {error}; audit: {outcome}"));
    assert_eq!(outcome["run"]["state"], "completed", "audit: {outcome}");
    assert_eq!(outcome["run"]["progress"]["assessed_rows"], 2);
    assert_eq!(outcome["report"]["row_count"], 2);
    assert!(outcome["report"].get("rows").is_none());
    assert_eq!(
        run_json(&database_url, ["quality", "assessments", &run_id])
            .as_array()
            .expect("assessment array")
            .len(),
        2
    );
}

#[test]
fn assessment_less_invalid_report_rows_can_be_explicitly_included_or_excluded() {
    let _serial = EXTERNAL_QUALITY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback provider");
    let address = listener.local_addr().expect("loopback address");
    listener
        .set_nonblocking(true)
        .expect("nonblocking provider listener");

    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("invalid-quality-row-review.db");
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
            "invalid-quality-row-review",
            "--task",
            "Classify support requests",
            "--label",
            "billing",
            "--label",
            "fraud",
        ],
    );
    let dataset_id = string_at(&dataset, "/id");
    let plan = run_json(
        &database_url,
        ["plan", "create", &dataset_id, "--per-cell", "1"],
    );
    let plan_id = string_at(&plan, "/id");
    run_json(&database_url, ["generate", &plan_id, "--backend", "fake"]);
    let source_row_ids = source_row_ids(&database_url, &dataset_id);
    assert_eq!(source_row_ids.len(), 2);

    run_json(
        &database_url,
        [
            "backend",
            "configure",
            "--base-url",
            &format!("http://{address}/v1"),
            "--model",
            "loopback-invalid-quality-model",
        ],
    );
    let audit = run_json(
        &database_url,
        [
            "quality",
            "audit-create",
            &dataset_id,
            "--preset",
            "fast",
            "--egress",
            "external-candidate-text",
            "--evaluator",
            "openai-compatible",
        ],
    );
    let run_id = string_at(&audit, "/audit_run/id");
    let provider = thread::spawn(move || serve_invalid_quality_responses(listener));
    let outcome = run_json(
        &database_url,
        [
            "quality",
            "audit-start",
            &run_id,
            "--api-key-env",
            "_ENCODER_GYM_TEST_MISSING_QUALITY_API_KEY",
        ],
    );
    assert_eq!(
        outcome["run"]["state"], "completed",
        "invalid-evaluator audit did not quarantine malformed output: {outcome}"
    );
    assert_eq!(outcome["run"]["progress"]["assessed_rows"], 0);
    assert_eq!(outcome["run"]["progress"]["invalid_rows"], 2);
    assert_eq!(outcome["report"]["totals"]["invalid_rows"], 2);
    assert!(outcome["report"].get("rows").is_none());
    let report_id = string_at(&outcome, "/report/id");
    assert!(
        run_json(&database_url, ["quality", "assessments", &run_id])
            .as_array()
            .expect("assessment array")
            .is_empty()
    );

    let include_review = run_json(
        &database_url,
        [
            "quality",
            "row-review",
            "--report-id",
            &report_id,
            "--source-row-id",
            &source_row_ids[0],
            "--include",
            "--reviewer",
            "invalid-row-operator",
            "--reason",
            "manual evidence supports including this row",
        ],
    );
    assert_eq!(include_review["decision"], "include");
    assert_eq!(include_review["report_id"], report_id);
    assert_eq!(include_review["source_row_id"], source_row_ids[0]);
    let exclude_review = run_json(
        &database_url,
        [
            "quality",
            "row-review",
            "--report-id",
            &report_id,
            "--source-row-id",
            &source_row_ids[1],
            "--exclude",
            "--reviewer",
            "invalid-row-operator",
            "--reason",
            "manual evidence supports excluding this row",
        ],
    );
    assert_eq!(exclude_review["decision"], "exclude");

    let proposal_summary = run_json(&database_url, ["quality", "curate", &run_id]);
    assert_eq!(proposal_summary["counts"]["included_rows"], 1);
    assert_eq!(proposal_summary["counts"]["excluded_rows"], 1);
    assert_eq!(proposal_summary["counts"]["needs_review_rows"], 0);
    assert!(proposal_summary.get("entries").is_none());
    let proposal_id = string_at(&proposal_summary, "/id");
    let proposal = run_json(&database_url, ["quality", "proposal", &proposal_id]);
    assert_entry_decision(&proposal, &source_row_ids[0], "include");
    assert_entry_decision(&proposal, &source_row_ids[1], "exclude");
    let included = proposal["entries"]
        .as_array()
        .expect("proposal entries")
        .iter()
        .find(|entry| entry["source_row_id"] == source_row_ids[0])
        .expect("included invalid row");
    assert_eq!(included["basis"], "human_include_override");
    assert!(
        included["assessment_references"]
            .as_array()
            .expect("assessment references")
            .is_empty()
    );
    assert!(
        !included["invalid_attempt_references"]
            .as_array()
            .expect("invalid attempt references")
            .is_empty()
    );

    run_json(
        &database_url,
        [
            "quality",
            "row-review",
            "--report-id",
            &report_id,
            "--source-row-id",
            &source_row_ids[0],
            "--include",
            "--reviewer",
            "invalid-row-operator",
            "--reason",
            "confirm inclusion after another evidence check",
        ],
    );
    let stale_approval = run(
        &database_url,
        [
            "quality",
            "manifest-review",
            &proposal_id,
            "--approve",
            "--reviewer",
            "invalid-row-operator",
            "--reason",
            "this stale proposal must not seal",
        ],
    );
    assert!(!stale_approval.status.success());
    assert!(
        String::from_utf8_lossy(&stale_approval.stderr).contains("rerun `quality curate"),
        "stale proposal rejection must explain the recovery path"
    );
    let refreshed_summary = run_json(&database_url, ["quality", "curate", &run_id]);
    assert_eq!(refreshed_summary["predecessor_id"], proposal_id);
    let refreshed_proposal_id = string_at(&refreshed_summary, "/id");

    let approved = run_json(
        &database_url,
        [
            "quality",
            "manifest-review",
            &refreshed_proposal_id,
            "--approve",
            "--reviewer",
            "invalid-row-operator",
            "--reason",
            "explicit row decisions are complete",
        ],
    );
    assert_eq!(approved["manifest"]["member_count"], 2);
    assert_eq!(approved["manifest"]["selected_member_count"], 1);
    assert_eq!(approved["manifest"]["excluded_member_count"], 1);
    assert!(approved["manifest"].get("members").is_none());

    provider
        .join()
        .expect("loopback provider thread")
        .expect("loopback invalid provider response");
}

#[test]
fn external_quality_audit_pins_backend_identity_and_rejects_policy_or_config_drift() {
    let _serial = EXTERNAL_QUALITY_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("external-quality-cli.db");
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
            "external-quality",
            "--task",
            "Classify support requests",
            "--label",
            "billing",
            "--label",
            "fraud",
        ],
    );
    let dataset_id = string_at(&dataset, "/id");
    let plan = run_json(
        &database_url,
        ["plan", "create", &dataset_id, "--per-cell", "1"],
    );
    let plan_id = string_at(&plan, "/id");
    run_json(&database_url, ["generate", &plan_id, "--backend", "fake"]);
    run_json(
        &database_url,
        [
            "backend",
            "configure",
            "--base-url",
            "http://127.0.0.1:9/v1",
            "--model",
            "quality-model-v1",
        ],
    );

    let rejected_independent_review = run(
        &database_url,
        [
            "quality",
            "audit-create",
            &dataset_id,
            "--preset",
            "balanced",
            "--egress",
            "external-candidate-text",
            "--evaluator",
            "openai-compatible",
        ],
    );
    assert!(!rejected_independent_review.status.success());
    assert!(
        String::from_utf8_lossy(&rejected_independent_review.stderr).contains(
            "openai-compatible quality audits currently support only --preset fast; balanced and strict require distinct reviewer model or backend configurations"
        )
    );

    let rejected_local = run(
        &database_url,
        [
            "quality",
            "audit-create",
            &dataset_id,
            "--preset",
            "fast",
            "--egress",
            "local-only",
            "--evaluator",
            "openai-compatible",
        ],
    );
    assert!(
        !rejected_local.status.success(),
        "core egress policy must reject an external evaluator"
    );

    let audit = run_json(
        &database_url,
        [
            "quality",
            "audit-create",
            &dataset_id,
            "--preset",
            "fast",
            "--egress",
            "external-candidate-text",
            "--evaluator",
            "openai-compatible",
        ],
    );
    let run_id = string_at(&audit, "/audit_run/id");
    assert_eq!(
        audit["audit_run"]["primary_evaluator"]["backend"],
        "openai-compatible"
    );
    assert_eq!(
        audit["audit_run"]["primary_evaluator"]["execution_location"],
        "external_service"
    );

    run_json(
        &database_url,
        [
            "backend",
            "configure",
            "--base-url",
            "http://127.0.0.1:9/v1",
            "--model",
            "quality-model-v2",
        ],
    );
    let rejected_drift = run(
        &database_url,
        [
            "quality",
            "audit-start",
            &run_id,
            "--api-key-env",
            "_ENCODER_GYM_TEST_MISSING_QUALITY_API_KEY",
        ],
    );
    assert!(!rejected_drift.status.success());
    assert!(
        String::from_utf8_lossy(&rejected_drift.stderr)
            .contains("do not exactly match the persisted audit run")
    );
}

#[test]
fn offline_quality_audit_curates_a_qualified_snapshot_and_detects_tampering() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("quality-cli.db");
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
            "quality-acceptance",
            "--task",
            "Classify support requests",
            "--label",
            "account security",
            "--label",
            "payment dispute",
            "--dimension",
            "difficulty=easy",
        ],
    );
    let dataset_id = string_at(&dataset, "/id");

    let semantics_path = directory.path().join("support-label-semantics.toml");
    std::fs::write(
        &semantics_path,
        r#"
schema_version = 1
key = "quality-support-labels"
description = "Stable label semantics pinned into the quality audit."

[scope]
kind = "reusable"

[target]
kind = "labels"

[entries."account security"]
description = "Unauthorized account access, password, or identity protection issues."
examples = ["Someone changed my password without permission."]

[entries."payment dispute"]
description = "Disputed, duplicate, or otherwise incorrect payment charges."
examples = ["I was charged twice for one purchase."]
"#,
    )
    .expect("semantic fixture");
    let semantic_profile = run_json(
        &database_url,
        [
            "semantic",
            "profile-create",
            semantics_path.to_str().expect("UTF-8 semantic path"),
        ],
    );
    let semantic_profile_id = string_at(&semantic_profile, "/id");
    run_json(
        &database_url,
        ["semantic", "bind", &dataset_id, &semantic_profile_id],
    );

    let generation_plan = run_json(
        &database_url,
        ["plan", "create", &dataset_id, "--per-cell", "1"],
    );
    let generation_plan_id = string_at(&generation_plan, "/id");
    let generation = run_json(
        &database_url,
        ["generate", &generation_plan_id, "--backend", "fake"],
    );
    assert_eq!(generation["state"], "completed");
    assert_eq!(generation["accepted_rows"], 2);

    let import_path = directory.path().join("realistic-support.csv");
    std::fs::write(
        &import_path,
        concat!(
            "text,label,difficulty\n",
            "\"An easy payment dispute about a duplicate charge.\",payment dispute,easy\n",
            "\"Someone accessed my account and changed the password.\",account security,easy\n",
        ),
    )
    .expect("import fixture");
    let imported = run_json(
        &database_url,
        [
            "dataset",
            "import",
            &dataset_id,
            "--input",
            import_path.to_str().expect("UTF-8 import path"),
            "--format",
            "csv",
            "--dimension",
            "difficulty=difficulty",
        ],
    );
    assert_eq!(imported["accepted_rows"], 2);
    assert_eq!(imported["rejected_rows"], 0);

    let legacy = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &dataset_id,
            "--name",
            "legacy-unqualified",
        ],
    );
    let legacy_snapshot_id = string_at(&legacy, "/snapshot/id");
    assert_eq!(legacy["snapshot"]["member_count"], 4);

    let audit = run_json(
        &database_url,
        [
            "quality",
            "audit-create",
            &dataset_id,
            "--preset",
            "fast",
            "--egress",
            "local-only",
            "--authenticity",
            "off",
            "--evaluator",
            "fake",
        ],
    );
    let audit_plan_id = string_at(&audit, "/audit_plan/id");
    let run_id = string_at(&audit, "/audit_run/id");
    assert_eq!(audit["audit_plan"]["population_rows"], 4);
    assert_eq!(audit["audit_plan"]["selected_rows"], 4);
    assert!(audit["audit_plan"].get("items").is_none());
    assert_eq!(audit["audit_run"]["state"], "queued");

    let outcome = run_json(&database_url, ["quality", "audit-start", &run_id]);
    assert_eq!(outcome["run"]["state"], "completed");
    let report_id = string_at(&outcome, "/report/id");

    let status = run_json(&database_url, ["quality", "audit-status", &run_id]);
    assert_eq!(status["run"]["state"], "completed");
    assert_eq!(status["plan"]["id"], audit_plan_id);
    assert_eq!(status["plan"]["population_rows"], 4);
    assert!(status["plan"].get("items").is_none());
    assert_eq!(status["assessments"], 4);
    assert_eq!(status["report_id"], report_id);

    let assessments = run_json(&database_url, ["quality", "assessments", &run_id]);
    let assessments = assessments.as_array().expect("assessment array");
    assert_eq!(assessments.len(), 4);
    assert!(assessments.iter().all(|assessment| {
        assessment["fingerprint"]
            .as_str()
            .is_some_and(|value| value.starts_with("sha256:"))
    }));
    let included_assessment_id = string_at(&assessments[0], "/id");
    let included_source_row_id = string_at(&assessments[0], "/source_row_id");
    let excluded_assessment_id = string_at(&assessments[1], "/id");
    let excluded_source_row_id = string_at(&assessments[1], "/source_row_id");
    assert_ne!(included_source_row_id, excluded_source_row_id);

    let summary = run_json(&database_url, ["quality", "summary", &run_id]);
    assert_eq!(summary["report_id"], report_id);
    assert_eq!(summary["totals"]["population_rows"], 4);
    assert_eq!(summary["totals"]["selected_rows"], 4);
    assert_eq!(summary["totals"]["assessed_rows"], 4);
    assert_eq!(summary["structurally_accepted_rows"], 4);
    assert_eq!(summary["population_rows"], 4);
    assert_eq!(summary["assessed_rows"], 4);
    assert_eq!(
        summary["remaining_qualified_rows"],
        summary["population_rows"]
            .as_u64()
            .expect("population count")
            - summary["qualified_rows"].as_u64().expect("qualified count")
    );

    let initial_proposal = run_json(&database_url, ["quality", "curate", &run_id]);
    let initial_proposal_id = string_at(&initial_proposal, "/id");
    assert_eq!(initial_proposal["predecessor_id"], Value::Null);
    assert_eq!(initial_proposal["counts"]["needs_review_rows"], 0);
    assert_eq!(initial_proposal["entry_count"], 4);
    assert!(initial_proposal.get("entries").is_none());

    let include_review = run_json(
        &database_url,
        [
            "quality",
            "row-review",
            &included_assessment_id,
            "--include",
            "--reviewer",
            "offline-operator",
            "--reason",
            "explicitly include this representative row",
        ],
    );
    assert_eq!(include_review["decision"], "include");
    let exclude_review = run_json(
        &database_url,
        [
            "quality",
            "row-review",
            &excluded_assessment_id,
            "--exclude",
            "--reviewer",
            "offline-operator",
            "--reason",
            "explicitly exclude this representative row",
        ],
    );
    assert_eq!(exclude_review["decision"], "exclude");
    for assessment in &assessments[2..] {
        let assessment_id = string_at(assessment, "/id");
        let review = run_json(
            &database_url,
            [
                "quality",
                "row-review",
                &assessment_id,
                "--exclude",
                "--reviewer",
                "offline-operator",
                "--reason",
                "exclude the remaining row from this acceptance manifest",
            ],
        );
        assert_eq!(review["decision"], "exclude");
    }

    let successor = run_json(&database_url, ["quality", "curate", &run_id]);
    let successor_id = string_at(&successor, "/id");
    assert_eq!(successor["predecessor_id"], initial_proposal_id);
    assert_eq!(successor["counts"]["needs_review_rows"], 0);
    assert_eq!(successor["counts"]["included_rows"], 1);
    assert_eq!(successor["counts"]["excluded_rows"], 3);
    assert!(successor.get("entries").is_none());
    let successor_detail = run_json(&database_url, ["quality", "proposal", &successor_id]);
    assert_entry_decision(&successor_detail, &included_source_row_id, "include");
    assert_entry_decision(&successor_detail, &excluded_source_row_id, "exclude");

    let approved = run_json(
        &database_url,
        [
            "quality",
            "manifest-review",
            &successor_id,
            "--approve",
            "--reviewer",
            "offline-operator",
            "--reason",
            "the exact reviewed proposal is approved",
        ],
    );
    assert_eq!(approved["review"]["decision"], "approve");
    let manifest_review_id = string_at(&approved, "/review/id");
    let stale_manifest_id = string_at(&approved, "/manifest/id");
    assert_eq!(approved["manifest"]["proposal_id"], successor_id);
    assert_eq!(approved["manifest"]["approval_id"], manifest_review_id);
    assert_eq!(approved["manifest"]["member_count"], 4);
    assert_eq!(approved["manifest"]["selected_member_count"], 1);
    assert!(approved["manifest"].get("members").is_none());
    assert_eq!(
        run_json(&database_url, ["quality", "manifest", &stale_manifest_id])["id"],
        stale_manifest_id
    );

    let later_review = run_json(
        &database_url,
        [
            "quality",
            "row-review",
            &excluded_assessment_id,
            "--exclude",
            "--reviewer",
            "later-offline-operator",
            "--reason",
            "new evidence reaffirms exclusion after the prior manifest approval",
        ],
    );
    assert_eq!(later_review["decision"], "exclude");
    let stale_application = run(
        &database_url,
        [
            "snapshot",
            "create",
            &dataset_id,
            "--name",
            "stale-manifest-must-not-apply",
            "--quality-manifest",
            &stale_manifest_id,
        ],
    );
    assert!(!stale_application.status.success());
    assert!(
        String::from_utf8_lossy(&stale_application.stderr).contains("is stale"),
        "stale manifest rejection must be actionable: {}",
        String::from_utf8_lossy(&stale_application.stderr)
    );

    let current_proposal = run_json(&database_url, ["quality", "curate", &run_id]);
    let current_proposal_id = string_at(&current_proposal, "/id");
    assert_eq!(current_proposal["predecessor_id"], successor_id);
    let current_proposal_detail =
        run_json(&database_url, ["quality", "proposal", &current_proposal_id]);
    let current_approval = run_json(
        &database_url,
        [
            "quality",
            "manifest-review",
            &current_proposal_id,
            "--approve",
            "--reviewer",
            "later-offline-operator",
            "--reason",
            "approve the exact successor containing the latest row review",
        ],
    );
    let manifest_id = string_at(&current_approval, "/manifest/id");

    let qualified = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &dataset_id,
            "--name",
            "qualified",
            "--quality-manifest",
            &manifest_id,
        ],
    );
    let qualified_snapshot_id = string_at(&qualified, "/snapshot/id");
    let application_id = string_at(&qualified, "/quality_application/id");
    assert_eq!(qualified["quality_manifest_id"], manifest_id);
    let qualified_replay = run_json(
        &database_url,
        [
            "snapshot",
            "create",
            &dataset_id,
            "--name",
            "qualified",
            "--quality-manifest",
            &manifest_id,
        ],
    );
    assert_eq!(
        string_at(&qualified_replay, "/snapshot/id"),
        qualified_snapshot_id
    );
    assert_eq!(
        string_at(&qualified_replay, "/quality_application/id"),
        application_id
    );

    let legacy_show = run_json(&database_url, ["snapshot", "show", &legacy_snapshot_id]);
    assert_eq!(legacy_show["qualified"], false);
    assert_eq!(legacy_show["curation_application_id"], Value::Null);
    assert_eq!(legacy_show["manifest_id"], Value::Null);
    let legacy_stats = run_json(&database_url, ["snapshot", "stats", &legacy_snapshot_id]);
    assert_eq!(legacy_stats["qualified"], false);
    assert_eq!(legacy_stats["curation_application_id"], Value::Null);
    assert_eq!(legacy_stats["manifest_id"], Value::Null);

    let qualified_show = run_json(&database_url, ["snapshot", "show", &qualified_snapshot_id]);
    assert_eq!(qualified_show["qualified"], true);
    assert_eq!(qualified_show["curation_application_id"], application_id);
    assert_eq!(qualified_show["manifest_id"], manifest_id);
    let qualified_stats = run_json(&database_url, ["snapshot", "stats", &qualified_snapshot_id]);
    assert_eq!(qualified_stats["qualified"], true);
    assert_eq!(qualified_stats["curation_application_id"], application_id);
    assert_eq!(qualified_stats["manifest_id"], manifest_id);

    let snapshots = run_json(
        &database_url,
        ["snapshot", "list", "--dataset-id", &dataset_id],
    );
    let snapshots = snapshots.as_array().expect("snapshot list");
    let legacy_listed = snapshots
        .iter()
        .find(|snapshot| snapshot["id"] == legacy_snapshot_id)
        .expect("legacy snapshot listed");
    assert_eq!(legacy_listed["qualified"], false);
    assert_eq!(legacy_listed["curation_application_id"], Value::Null);
    assert_eq!(legacy_listed["manifest_id"], Value::Null);
    let qualified_listed = snapshots
        .iter()
        .find(|snapshot| snapshot["id"] == qualified_snapshot_id)
        .expect("qualified snapshot listed");
    assert_eq!(qualified_listed["qualified"], true);
    assert_eq!(qualified_listed["curation_application_id"], application_id);
    assert_eq!(qualified_listed["manifest_id"], manifest_id);

    let expected_included = proposal_members(&current_proposal_detail, "include");
    let expected_excluded = proposal_members(&current_proposal_detail, "exclude");
    assert!(!expected_included.is_empty());
    assert!(!expected_excluded.is_empty());
    let qualified_members = run_json(
        &database_url,
        ["snapshot", "members", &qualified_snapshot_id],
    );
    assert_eq!(member_ids(&qualified_members), expected_included);
    let legacy_members = run_json(&database_url, ["snapshot", "members", &legacy_snapshot_id]);
    assert_eq!(legacy_members.as_array().expect("legacy members").len(), 4);
    assert!(expected_included.is_subset(&member_ids(&legacy_members)));

    let application_provenance = run_json(
        &database_url,
        ["provenance", "curation-application", &application_id],
    );
    let application_kinds = provenance_kinds(&application_provenance);
    for expected in [
        "curation_application",
        "approved_curation_manifest",
        "curation_manifest_review",
        "curation_proposal",
        "row_quality_review",
        "dataset_quality_report",
        "row_quality_assessment",
        "quality_evaluator_attempt",
        "quality_audit_run",
        "quality_audit_plan",
        "quality_semantic_guidance",
        "dataset_source_row",
    ] {
        assert!(
            application_kinds.contains(expected),
            "application provenance omitted {expected}: {application_kinds:?}"
        );
    }
    let semantic_guidance_provenance = run_json(
        &database_url,
        ["provenance", "quality-semantic-guidance", &audit_plan_id],
    );
    assert_eq!(
        semantic_guidance_provenance["kind"],
        "quality_semantic_guidance"
    );
    assert!(
        provenance_kinds(&semantic_guidance_provenance).contains("semantic_binding"),
        "quality semantic guidance must link its immutable semantic binding"
    );
    let qualified_provenance = run_json(
        &database_url,
        ["provenance", "snapshot", &qualified_snapshot_id],
    );
    assert!(
        provenance_kinds(&qualified_provenance).contains("curation_application"),
        "qualified snapshot must expose its curation application"
    );
    let legacy_provenance = run_json(
        &database_url,
        ["provenance", "snapshot", &legacy_snapshot_id],
    );
    assert!(
        !provenance_kinds(&legacy_provenance).contains("curation_application"),
        "legacy snapshot must remain visibly unqualified"
    );

    let doctor = run_json(&database_url, ["doctor"]);
    assert_eq!(doctor["healthy"], true);
    assert!(doctor_check(&doctor, "dataset_quality_facts", "pass"));

    tamper_report(&database_url, &report_id);
    let rejected_load = run(&database_url, ["quality", "summary", &run_id]);
    assert!(!rejected_load.status.success());
    assert!(
        !rejected_load.stderr.is_empty(),
        "tampered report load must explain its rejection"
    );
    let rejected_doctor = run(&database_url, ["doctor"]);
    assert!(!rejected_doctor.status.success());
    let rejected_doctor: Value =
        serde_json::from_slice(&rejected_doctor.stdout).expect("doctor JSON after tampering");
    assert_eq!(rejected_doctor["healthy"], false);
    assert!(doctor_check(
        &rejected_doctor,
        "dataset_quality_facts",
        "fail"
    ));
}

fn string_at(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string at {pointer}: {value}"))
        .to_owned()
}

fn assert_entry_decision(proposal: &Value, source_row_id: &str, decision: &str) {
    let entry = proposal["entries"]
        .as_array()
        .expect("proposal entries")
        .iter()
        .find(|entry| entry["source_row_id"] == source_row_id)
        .unwrap_or_else(|| panic!("proposal has no row {source_row_id}"));
    assert_eq!(entry["decision"], decision);
    assert!(entry["applied_row_review_id"].is_string());
}

fn proposal_members(proposal: &Value, decision: &str) -> BTreeSet<String> {
    proposal["entries"]
        .as_array()
        .expect("proposal entries")
        .iter()
        .filter(|entry| entry["decision"] == decision)
        .map(|entry| string_at(entry, "/source_row_id"))
        .collect()
}

fn member_ids(members: &Value) -> BTreeSet<String> {
    members
        .as_array()
        .expect("snapshot members")
        .iter()
        .map(|member| string_at(member, "/source_row_id"))
        .collect()
}

fn provenance_kinds(trace: &Value) -> BTreeSet<String> {
    fn visit(value: &Value, values: &mut BTreeSet<String>) {
        if let Some(kind) = value["kind"].as_str() {
            values.insert(kind.to_owned());
        }
        if let Some(parents) = value["parents"].as_array() {
            for parent in parents {
                visit(parent, values);
            }
        }
    }

    let mut values = BTreeSet::new();
    visit(trace, &mut values);
    values
}

fn doctor_check(report: &Value, name: &str, status: &str) -> bool {
    report["checks"]
        .as_array()
        .expect("doctor checks")
        .iter()
        .any(|check| check["name"] == name && check["status"] == status)
}

fn tamper_report(database_url: &str, report_id: &str) {
    let report_id = Uuid::parse_str(report_id).expect("report UUID");
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let pool = sqlx::SqlitePool::connect(database_url)
                .await
                .expect("database connects");
            let report_json: String =
                sqlx::query_scalar("SELECT report_json FROM dataset_quality_reports WHERE id = ?")
                    .bind(report_id)
                    .fetch_one(&pool)
                    .await
                    .expect("report JSON loads");
            let mut report: Value =
                serde_json::from_str(&report_json).expect("report JSON decodes");
            let qualified = report["totals"]["qualified_rows"]
                .as_u64()
                .expect("qualified count");
            report["totals"]["qualified_rows"] = serde_json::json!(qualified + 1);
            sqlx::query("UPDATE dataset_quality_reports SET report_json = ? WHERE id = ?")
                .bind(serde_json::to_string(&report).expect("report JSON encodes"))
                .bind(report_id)
                .execute(&pool)
                .await
                .expect("report JSON tampers");
            pool.close().await;
        });
}

fn source_row_ids(database_url: &str, dataset_id: &str) -> Vec<String> {
    let dataset_id = Uuid::parse_str(dataset_id).expect("dataset UUID");
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(async {
            let pool = sqlx::SqlitePool::connect(database_url)
                .await
                .expect("database connects");
            let values = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM dataset_source_rows WHERE dataset_id = ? ORDER BY id",
            )
            .bind(dataset_id)
            .fetch_all(&pool)
            .await
            .expect("source row IDs load")
            .into_iter()
            .map(|value| value.to_string())
            .collect();
            pool.close().await;
            values
        })
}

fn serve_one_quality_response(listener: TcpListener) -> Result<(), String> {
    let mut stream = accept_quality_connection(&listener)?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| format!("provider read timeout failed: {error}"))?;
    let raw = read_http_request(&mut stream)?;
    let header_end = find_bytes(&raw, b"\r\n\r\n")
        .ok_or_else(|| "provider request omitted the header terminator".to_owned())?
        + 4;
    let request: Value = serde_json::from_slice(&raw[header_end..])
        .map_err(|error| format!("provider request JSON failed: {error}"))?;
    let prompt: Value = serde_json::from_str(
        request["messages"][1]["content"]
            .as_str()
            .ok_or_else(|| "provider request omitted the blind user message".to_owned())?,
    )
    .map_err(|error| format!("blind prompt JSON failed: {error}"))?;
    let labels = prompt["allowed_labels"]
        .as_array()
        .ok_or_else(|| "blind prompt omitted labels".to_owned())?;
    let dimensions = prompt["allowed_dimensions"]
        .as_object()
        .ok_or_else(|| "blind prompt omitted dimensions".to_owned())?;
    let assessments = prompt["candidate_rows"]
        .as_array()
        .ok_or_else(|| "blind prompt omitted rows".to_owned())?
        .iter()
        .map(|row| {
            let label_scores = labels
                .iter()
                .enumerate()
                .map(|(index, label)| {
                    (
                        label.as_str().expect("label string").to_owned(),
                        serde_json::json!(if index == 0 { 9_000 } else { 1_000 }),
                    )
                })
                .collect::<serde_json::Map<_, _>>();
            let dimension_scores = dimensions
                .iter()
                .map(|(name, values)| {
                    let scores = values
                        .as_array()
                        .expect("dimension values")
                        .iter()
                        .map(|value| {
                            (
                                value.as_str().expect("dimension value").to_owned(),
                                serde_json::json!(9_000),
                            )
                        })
                        .collect::<serde_json::Map<_, _>>();
                    (name.clone(), Value::Object(scores))
                })
                .collect::<serde_json::Map<_, _>>();
            serde_json::json!({
                "source_row_id": row["source_row_id"],
                "source_row_fingerprint": row["source_row_fingerprint"],
                "label_scores": label_scores,
                "dimension_scores": dimension_scores,
                "label_leakage_risk": 100,
                "shortcut_risk": 100,
                "confidence": 9_500,
                "issue_codes": [],
                "rationale": "Loopback evaluator assessed the candidate blindly."
            })
        })
        .collect::<Vec<_>>();
    let content = serde_json::json!({"assessments": assessments}).to_string();
    write_quality_response(&mut stream, "loopback-quality-model", content, 50)
}

fn serve_invalid_quality_responses(listener: TcpListener) -> Result<(), String> {
    // The fast policy retries malformed evaluator output once before
    // quarantining it. Keep the provider alive for the complete persisted
    // attempt budget so this test never turns an invalid-output case into a
    // transport failure on the retry.
    for attempt in 0..2 {
        let stream = if attempt == 0 {
            Some(accept_quality_connection(&listener)?)
        } else {
            accept_optional_quality_connection(&listener, Duration::from_secs(5))?
        };
        let Some(mut stream) = stream else {
            break;
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(|error| format!("provider read timeout failed: {error}"))?;
        read_http_request(&mut stream)?;
        write_quality_response(
            &mut stream,
            "loopback-invalid-quality-model",
            serde_json::json!({"assessments": []}).to_string(),
            10,
        )?;
    }
    Ok(())
}

fn accept_optional_quality_connection(
    listener: &TcpListener,
    timeout: Duration,
) -> Result<Option<TcpStream>, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream
                    .set_nonblocking(false)
                    .map_err(|error| format!("provider blocking stream failed: {error}"))?;
                return Ok(Some(stream));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Ok(None);
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("provider accept failed: {error}")),
        }
    }
}

fn accept_quality_connection(listener: &TcpListener) -> Result<TcpStream, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let (stream, _) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("quality evaluator never connected".into());
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(format!("provider accept failed: {error}")),
        }
    };
    // Windows accepted sockets can inherit the listener's nonblocking mode.
    // The request reader uses a bounded blocking read, not readiness polling.
    stream
        .set_nonblocking(false)
        .map_err(|error| format!("provider blocking stream failed: {error}"))?;
    Ok(stream)
}

fn write_quality_response(
    stream: &mut TcpStream,
    model: &str,
    content: String,
    completion_tokens: u64,
) -> Result<(), String> {
    let response = serde_json::json!({
        "id": "loopback-quality-response",
        "object": "chat.completion",
        "created": 1,
        "model": model,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content, "refusal": null},
            "finish_reason": "stop",
            "logprobs": null
        }],
        "usage": {
            "prompt_tokens": 100,
            "completion_tokens": completion_tokens,
            "total_tokens": 100 + completion_tokens
        }
    })
    .to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
        response.len(),
        response
    )
    .map_err(|error| format!("provider response failed: {error}"))?;
    stream
        .flush()
        .map_err(|error| format!("provider flush failed: {error}"))
}

fn read_http_request(stream: &mut impl Read) -> Result<Vec<u8>, String> {
    let mut raw = Vec::new();
    let mut expected_length = None;
    loop {
        let mut chunk = [0_u8; 8 * 1024];
        let read = stream
            .read(&mut chunk)
            .map_err(|error| format!("provider request read failed: {error}"))?;
        if read == 0 {
            break;
        }
        raw.extend_from_slice(&chunk[..read]);
        if expected_length.is_none()
            && let Some(header_index) = find_bytes(&raw, b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&raw[..header_index]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(str::trim)
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .ok_or_else(|| "provider request omitted content-length".to_owned())?;
            expected_length = Some(header_index + 4 + content_length);
        }
        if expected_length.is_some_and(|length| raw.len() >= length) {
            return Ok(raw);
        }
    }
    Err("provider request ended before its declared body".into())
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
