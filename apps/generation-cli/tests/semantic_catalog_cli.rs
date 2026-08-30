#[allow(dead_code)]
mod support;

use support::run_json;

#[test]
fn semantic_profiles_resolve_into_generation_and_remain_version_pinned() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let database = directory.path().join("semantics.db");
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
            "Classify support requests",
            "--label",
            "billing",
            "--label",
            "fraud",
            "--dimension",
            "difficulty=easy,hard",
        ],
    );
    let dataset_id = dataset["id"].as_str().expect("dataset id").to_owned();

    let profile_v1_path = directory.path().join("difficulty-v1.toml");
    std::fs::write(
        &profile_v1_path,
        r#"
schema_version = 1
key = "support-difficulty"
description = "Difficulty means how indirect the intent is, not vocabulary complexity."

[scope]
kind = "reusable"

[target]
kind = "dimension"
name = "difficulty"

[entries.easy]
description = "The intent is stated directly."
examples = ["Why was I charged twice?"]
inclusion_rules = ["A single explicit clue is sufficient."]

[entries.hard]
description = "The intent must be inferred from multiple clues."
counterexamples = ["Do not make text hard merely by adding rare words."]
exclusion_rules = ["No direct category keyword."]
"#,
    )
    .expect("profile v1");
    let profile_v1 = run_json(
        &database_url,
        [
            "semantic",
            "profile-create",
            profile_v1_path.to_str().expect("path"),
        ],
    );
    let profile_v1_id = profile_v1["id"].as_str().expect("profile id").to_owned();
    assert_eq!(profile_v1["version"], 1);

    let binding_v1 = run_json(
        &database_url,
        ["semantic", "bind", &dataset_id, &profile_v1_id],
    );
    assert_eq!(binding_v1["profile_id"], profile_v1_id);
    let resolved_v1 = run_json(&database_url, ["semantic", "resolve", &dataset_id]);
    assert_eq!(resolved_v1["sources"][0]["profile_version"], 1);

    let plan = run_json(
        &database_url,
        ["plan", "create", &dataset_id, "--per-cell", "1"],
    );
    let plan_id = plan["id"].as_str().expect("plan id").to_owned();
    let first_job = run_json(&database_url, ["generate", &plan_id, "--backend", "fake"]);
    let first_job_id = first_job["id"].as_str().expect("job id").to_owned();
    assert_eq!(first_job["state"], "completed");
    let first_context = run_json(&database_url, ["semantic", "job-context", &first_job_id]);
    assert_eq!(first_context["context"]["sources"][0]["profile_version"], 1);

    let generated_rows = run_json(&database_url, ["rows", "--job-id", &first_job_id]);
    assert_eq!(
        generated_rows[0]["generation_metadata"]["semantic_context"]["sources"][0]["profile_version"],
        1
    );

    let profile_v2_path = directory.path().join("difficulty-v2.toml");
    std::fs::write(
        &profile_v2_path,
        r#"
schema_version = 1
key = "support-difficulty"
description = "Revised definition for future jobs only."

[scope]
kind = "reusable"

[target]
kind = "dimension"
name = "difficulty"

[entries.easy]
description = "Direct and unambiguous intent."

[entries.hard]
description = "Indirect intent requiring multiple clues."
"#,
    )
    .expect("profile v2");
    let profile_v2 = run_json(
        &database_url,
        [
            "semantic",
            "profile-revise",
            &profile_v1_id,
            profile_v2_path.to_str().expect("path"),
        ],
    );
    let profile_v2_id = profile_v2["id"].as_str().expect("profile id").to_owned();
    assert_eq!(profile_v2["version"], 2);
    run_json(
        &database_url,
        ["semantic", "bind", &dataset_id, &profile_v2_id],
    );
    let resolved_v2 = run_json(&database_url, ["semantic", "resolve", &dataset_id]);
    assert_eq!(resolved_v2["sources"][0]["profile_version"], 2);

    let still_pinned = run_json(&database_url, ["semantic", "job-context", &first_job_id]);
    assert_eq!(still_pinned["context"]["sources"][0]["profile_version"], 1);

    let provenance = run_json(
        &database_url,
        ["provenance", "generation-semantic-context", &first_job_id],
    );
    assert_eq!(provenance["kind"], "generation_semantic_context");
    assert_eq!(provenance["parents"][0]["kind"], "semantic_binding");
    assert_eq!(
        provenance["parents"][0]["parents"][1]["kind"],
        "semantic_profile"
    );

    let doctor = run_json(&database_url, ["doctor"]);
    assert_eq!(doctor["healthy"], true);
    assert!(
        doctor["checks"]
            .as_array()
            .expect("checks")
            .iter()
            .any(|check| {
                check["name"] == "semantic_catalog_facts" && check["status"] == "pass"
            })
    );
}
