pub mod support;

use std::path::Path;
use support::{
    run, run_json,
    workflow_fixture::{GenerationMode, WorkflowFixture},
};

fn url(path: &Path) -> String {
    format!(
        "sqlite://{}?mode=rwc",
        path.to_string_lossy().replace('\\', "/")
    )
}

#[test]
fn pure_configuration_and_policy_commands_never_open_a_database() {
    let directory = tempfile::tempdir().unwrap();
    let database_url = url(&directory.path().join("missing-parent/database.db"));
    let file = directory.path().join("project.toml");
    std::fs::write(&file, include_str!("fixtures/hybrid-project.toml")).unwrap();
    let file = file.to_str().unwrap();
    assert_eq!(
        run_json(&database_url, ["config", "validate", file])["valid"],
        true
    );
    run_json(&database_url, ["config", "show", file]);
    run_json(
        &database_url,
        [
            "config",
            "construction-preview",
            file,
            "--label",
            "billing",
            "--dimension",
            "style=clean",
        ],
    );
    run_json(&database_url, ["quality", "policy-preview"]);
    let brief = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/benchmark-architect/support-benchmark-brief.json");
    run_json(
        &database_url,
        [
            "benchmark-architect",
            "brief-validate",
            brief.to_str().unwrap(),
        ],
    );
    assert!(!directory.path().join("missing-parent").exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[test]
fn passive_missing_database_fails_without_creating_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("passive.db");
    let database_url = url(&path);
    for args in [
        vec!["dataset", "list"],
        vec!["doctor"],
        vec!["recovery", "list"],
    ] {
        assert!(!run(&database_url, args).status.success());
        assert!(!path.exists());
    }
    assert_eq!(
        run_json(&database_url, ["database", "migrate"])["migrated"],
        true
    );
    assert!(path.exists());
    run_json(&database_url, ["dataset", "list"]);
}

#[test]
fn preview_and_status_leave_interrupted_work_untouched_until_explicit_scan() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let prepared = fixture.prepare();
    let definition = prepared["preparation"]["workflow_definition_id"]
        .as_str()
        .unwrap();
    let started = run_json(
        fixture.database_url(),
        ["workflow", "start", definition, "--initialize-only"],
    );
    let id = started["run"]["id"].as_str().unwrap();
    assert_eq!(started["run"]["state"], "running");
    run_json(
        fixture.database_url(),
        [
            "project",
            "preview",
            fixture.manifest_path().to_str().unwrap(),
        ],
    );
    let inspected = run_json(fixture.database_url(), ["workflow", "status", id]);
    assert_eq!(inspected["run"]["state"], "running");
    assert_eq!(
        run_json(fixture.database_url(), ["recovery", "list"]),
        serde_json::json!([])
    );
    let detected = run_json(fixture.database_url(), ["recovery", "scan"]);
    assert!(
        detected
            .as_array()
            .unwrap()
            .iter()
            .any(|record| record["workflow_id"] == id)
    );
    assert_eq!(
        run_json(fixture.database_url(), ["recovery", "scan"]),
        serde_json::json!([])
    );
}

#[test]
fn passive_cli_works_with_a_read_only_file_and_preserves_its_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("passive.db");
    let database_url = url(&path);
    run_json(&database_url, ["database", "migrate"]);
    // DELETE mode also proves startup does not silently switch the database to WAL.
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let pool = sqlx::SqlitePool::connect(&database_url).await.unwrap();
        sqlx::query("PRAGMA journal_mode=DELETE")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
    });
    let original_permissions = std::fs::metadata(&path).unwrap().permissions();
    let mut read_only = original_permissions.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(&path, read_only).unwrap();
    let before = std::fs::read(&path).unwrap();
    let list = run(&database_url, ["dataset", "list"]);
    let doctor = run(&database_url, ["doctor"]);
    let after = std::fs::read(&path).unwrap();
    std::fs::set_permissions(&path, original_permissions).unwrap();
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    assert!(
        doctor.status.success(),
        "{}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert_eq!(before, after);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}
