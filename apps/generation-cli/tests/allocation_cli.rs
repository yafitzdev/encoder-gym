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
    let space = run_json(&database_url, ["plan", "describe", dataset_id]);
    assert_eq!(space["label_count"], 2);
    assert_eq!(space["generation_cell_count"], 4);

    let constraints_path = directory.path().join("constraints.toml");
    std::fs::write(
        &constraints_path,
        r#"
[[rules]]
minimum_target = 3

[rules.selector]
label = "billing"

[rules.selector.dimensions]
difficulty = "hard"
"#,
    )
    .expect("constraints");
    let constraints = constraints_path.to_str().expect("constraint path");

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

    let explained_preview = run_json(
        &database_url,
        [
            "allocation",
            "preview",
            dataset_id,
            "--total-rows",
            "10",
            "--constraints",
            constraints,
            "--explain",
        ],
    );
    assert_eq!(explained_preview["allocation"]["feasibility"], "feasible");
    assert_eq!(
        explained_preview["explanation"]["minimum_constrained_cell_count"],
        1
    );
    assert_eq!(
        explained_preview["explanation"]["labels"]
            .as_array()
            .expect("labels")
            .iter()
            .map(|label| label["target_rows"].as_u64().expect("target"))
            .sum::<u64>(),
        10
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
            "--constraints",
            constraints,
        ],
    );
    let allocation_id = created["id"].as_str().expect("allocation ID");
    let plan_id = created["generation_plan_id"].as_str().expect("plan ID");
    assert_eq!(created["result"]["initial_target_rows"], 8);
    assert_eq!(
        created["result"]["cells"]
            .as_array()
            .expect("cells")
            .iter()
            .filter(|cell| cell["minimum_target"] == 3)
            .count(),
        1
    );

    let explanation = run_json(&database_url, ["allocation", "explain", allocation_id]);
    assert_eq!(explanation["generation_cell_count"], 4);
    assert_eq!(explanation["minimum_constrained_cell_count"], 1);

    let provenance = run_json(
        &database_url,
        ["provenance", "initial-allocation", allocation_id],
    );
    assert_eq!(provenance["kind"], "initial_allocation");
    assert_eq!(provenance["parents"][0]["kind"], "dataset");

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
    let plan_provenance = run_json(&database_url, ["provenance", "generation-plan", plan_id]);
    assert!(
        plan_provenance["parents"]
            .as_array()
            .expect("plan parents")
            .iter()
            .any(|parent| parent["kind"] == "initial_allocation")
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
