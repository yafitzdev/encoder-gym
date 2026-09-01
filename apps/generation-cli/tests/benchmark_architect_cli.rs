#[allow(dead_code)]
mod support;

use std::path::PathBuf;

use serde_json::{Value, json};
use tempfile::TempDir;

use support::{run, run_json};

#[test]
fn full_offline_pi_benchmark_architecture_review_handoff_and_conformance() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let sidecar = root.join("adapters/research-agent-pi/dist/main.js");
    if !sidecar.is_file() {
        eprintln!(
            "skipping process-level Pi acceptance because {} is not built",
            sidecar.display()
        );
        return;
    }
    let temporary = tempfile::tempdir().expect("temporary directory");
    let database_url = sqlite_url(&temporary, "benchmark-architect-cli.db");
    let brief = path(
        &root,
        "examples/benchmark-architect/support-benchmark-brief.json",
    );
    let script = path(
        &root,
        "examples/benchmark-architect/support-scripted-turns.json",
    );
    let corpus = path(&root, "examples/benchmark-architect/support-corpus.json");
    let sidecar = sidecar.to_string_lossy().into_owned();
    let outcome = run_json(
        &database_url,
        [
            "benchmark-architect",
            "start",
            &brief,
            "--script",
            &script,
            "--corpus",
            &corpus,
            "--pi-sidecar",
            &sidecar,
        ],
    );
    assert_eq!(outcome["run"]["state"], "awaiting_review");
    assert_eq!(outcome["run"]["usage"]["blueprint_previews"], 1);
    let run_id = text(&outcome["run"], "id");
    let proposal_id = text(&outcome["proposal"], "id");
    let status = run_json(&database_url, ["benchmark-architect", "status", &run_id]);
    assert_eq!(status["evidence_count"], 1);
    for call in status["tool_calls"].as_array().expect("tool calls") {
        if call["kind"] == "fetch_page" {
            assert!(call["response"].get("content").is_none());
        }
    }

    let review = run_json(
        &database_url,
        [
            "benchmark-architect",
            "review",
            &proposal_id,
            "--approve",
            "--reason",
            "offline acceptance",
        ],
    );
    assert_eq!(review["decision"], "approve");
    let handoff = run_json(
        &database_url,
        ["benchmark-architect", "handoff", &proposal_id],
    );
    let handoff_id = text(&handoff, "id");
    let repeated = run_json(
        &database_url,
        ["benchmark-architect", "handoff", &proposal_id],
    );
    assert_eq!(repeated["id"], handoff["id"]);
    assert_eq!(repeated["fingerprint"], handoff["fingerprint"]);
    assert!(
        handoff["authority_notice"]
            .as_str()
            .expect("authority notice")
            .contains("does not authorize")
    );
    let shown = run_json(
        &database_url,
        ["benchmark-architect", "handoff-show", &handoff_id],
    );
    assert_eq!(shown["fingerprint"], handoff["fingerprint"]);

    let facts_path = temporary.path().join("acquisition-facts.json");
    let protocol = handoff["requirements"][0]["protocol"].clone();
    let acquired_at = chrono::Utc::now();
    let candidate = |key: &str, kind: &str, role: &str, source: &str| {
        json!({
            "cohort_key": key,
            "suite_kind": kind,
            "origin": "internal_snapshot",
            "role": role,
            "disclosure": "aggregate",
            "adaptation_eligible": kind == "development",
            "protocol": protocol.clone(),
            "total_support": 400,
            "label_support": {"billing": 200, "fraud": 200},
            "dimension_value_support": {"writing_style": {"messy": 400}},
            "slice_support": {},
            "source_classes": [source],
            "distinct_producers": 2,
            "acquired_at": acquired_at,
            "workflow_iterations": 0,
            "adaptive_exposures": 0,
            "acceptance_exposures": 0,
            "retired": false
        })
    };
    std::fs::write(
        &facts_path,
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "candidates": [
                candidate("dev-authentic-boundary", "development", "development", "reviewed_real_sample"),
                candidate("sealed-authentic-boundary", "sealed_acceptance", "sealed_acceptance", "independent_reviewed_benchmark")
            ]
        }))
        .expect("serialize facts"),
    )
    .expect("write facts");
    let facts_path = facts_path.to_string_lossy().into_owned();
    let conformance = run_json(
        &database_url,
        [
            "benchmark-architect",
            "conformance",
            &handoff_id,
            "--file",
            &facts_path,
        ],
    );
    assert_eq!(conformance["state"], "conformant");

    let provenance = run_json(
        &database_url,
        ["provenance", "benchmark-acquisition-handoff", &handoff_id],
    );
    assert_eq!(provenance["kind"], "benchmark_acquisition_handoff");
    let doctor = run_json(&database_url, ["doctor"]);
    assert_eq!(doctor["healthy"], true);

    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    runtime.block_on(async {
        let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();
        sqlx::query("DROP TRIGGER benchmark_architecture_proposals_immutable")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "UPDATE benchmark_architecture_proposals SET proposal_json = '{}' WHERE id = ?",
        )
        .bind(uuid::Uuid::parse_str(&proposal_id).unwrap())
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    });
    let tampered = run(&database_url, ["doctor"]);
    assert!(!tampered.status.success());
    let report: Value = serde_json::from_slice(&tampered.stdout).unwrap();
    assert_eq!(report["healthy"], false);
    assert!(report["checks"].as_array().unwrap().iter().any(|check| {
        check["name"] == "benchmark_architect_facts" && check["status"] == "fail"
    }));
}

#[test]
fn queued_cancellation_and_interrupted_recovery_are_durable() {
    use benchmark_architect_core::{
        brief::{BenchmarkArchitectBriefDraft, ResolvedBenchmarkArchitectBrief},
        lifecycle::{
            BenchmarkArchitectRun, BenchmarkArchitectToolCall, BenchmarkArchitectToolKind,
        },
        ports::BenchmarkArchitectStore,
    };
    use synthetic_data_sqlite::SqliteStore;

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temporary = tempfile::tempdir().expect("temporary directory");
    let database_url = sqlite_url(&temporary, "benchmark-architect-control.db");
    let (queued_id, running_id) =
        tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                let store = SqliteStore::connect(&database_url).await.unwrap();
                let mut value: Value = serde_json::from_slice(
                    &std::fs::read(
                        root.join("examples/benchmark-architect/support-benchmark-brief.json"),
                    )
                    .unwrap(),
                )
                .unwrap();
                value.as_object_mut().unwrap().remove("schema_version");
                let draft: BenchmarkArchitectBriefDraft = serde_json::from_value(value).unwrap();
                let first = ResolvedBenchmarkArchitectBrief::create(draft.clone()).unwrap();
                let queued = BenchmarkArchitectRun::queue(&first, 1, "sha256:test".into()).unwrap();
                BenchmarkArchitectStore::create_run(&store, &first, &queued)
                    .await
                    .unwrap();

                let second = ResolvedBenchmarkArchitectBrief::create(draft).unwrap();
                let mut running =
                    BenchmarkArchitectRun::queue(&second, 1, "sha256:test".into()).unwrap();
                BenchmarkArchitectStore::create_run(&store, &second, &running)
                    .await
                    .unwrap();
                running.start().unwrap();
                BenchmarkArchitectStore::save_run(&store, &running)
                    .await
                    .unwrap();
                let call = BenchmarkArchitectToolCall::start(
                    &running,
                    1,
                    BenchmarkArchitectToolKind::InspectBrief,
                    json!({}),
                )
                .unwrap();
                BenchmarkArchitectStore::record_tool_call(&store, &call)
                    .await
                    .unwrap();
                let ids = (queued.id.to_string(), running.id.to_string());
                store.pool().close().await;
                ids
            });

    let cancelled = run_json(&database_url, ["benchmark-architect", "cancel", &queued_id]);
    assert_eq!(cancelled["state"], "cancelled");
    let recovered = run_json(
        &database_url,
        ["benchmark-architect", "recover", &running_id],
    );
    assert_eq!(recovered["state"], "failed");
    assert_eq!(recovered["stop_reason"], "interrupted");
    let status = run_json(
        &database_url,
        ["benchmark-architect", "status", &running_id],
    );
    assert_eq!(status["tool_calls"][0]["state"], "interrupted");
    assert_eq!(run_json(&database_url, ["doctor"])["healthy"], true);
}

fn text(value: &Value, field: &str) -> String {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} was not a string in {value}"))
        .to_owned()
}

fn path(root: &std::path::Path, relative: &str) -> String {
    root.join(relative).to_string_lossy().into_owned()
}

fn sqlite_url(directory: &TempDir, name: &str) -> String {
    format!(
        "sqlite://{}?mode=rwc",
        directory
            .path()
            .join(name)
            .to_string_lossy()
            .replace('\\', "/")
    )
}
