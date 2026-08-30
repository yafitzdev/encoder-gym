pub mod support;

use support::run_json;

#[test]
fn exact_initial_budget_is_previewed_and_persisted_as_an_ordinary_plan() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("allocation.db");
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
            "support",
            "--task",
            "classify support requests",
            "--label",
            "billing",
            "--label",
            "fraud",
            "--dimension",
            "difficulty=easy,hard",
        ],
    );
    let dataset_id = dataset["id"].as_str().expect("dataset ID");

    let preview = run_json(
        &database_url,
        [
            "allocation",
            "preview",
            dataset_id,
            "--total-rows",
            "10",
            "--reserved-rows",
            "2",
        ],
    );
    assert_eq!(preview["requested_total_rows"], 10);
    assert_eq!(preview["initial_target_rows"], 8);
    assert_eq!(preview["allocated_target_rows"], 8);
    assert_eq!(preview["feasibility"], "feasible");
    assert!(
        preview["fingerprint"]
            .as_str()
            .expect("fingerprint")
            .starts_with("sha256:")
    );
    assert_eq!(
        run_json(
            &database_url,
            ["allocation", "list", "--dataset-id", dataset_id]
        ),
        serde_json::json!([]),
        "preview must be read-only"
    );

    let created = run_json(
        &database_url,
        [
            "allocation",
            "create",
            dataset_id,
            "--total-rows",
            "10",
            "--reserved-rows",
            "2",
        ],
    );
    let allocation_id = created["id"].as_str().expect("allocation ID");
    let plan_id = created["generation_plan_id"].as_str().expect("plan ID");
    assert_eq!(created["result"]["initial_target_rows"], 8);

    let plan = run_json(&database_url, ["plan", "show", plan_id]);
    assert_eq!(
        plan["cells"]
            .as_array()
            .expect("cells")
            .iter()
            .map(|cell| cell["target_count"].as_u64().expect("target"))
            .sum::<u64>(),
        8
    );
    assert_eq!(
        run_json(&database_url, ["allocation", "show", allocation_id]),
        created
    );
    assert_eq!(
        run_json(
            &database_url,
            ["allocation", "list", "--dataset-id", dataset_id]
        )
        .as_array()
        .expect("allocations")
        .len(),
        1
    );
}
