pub mod support;

use workflow_core::{
    governance::{CohortRole, DisclosureLevel},
    workflow::{ApprovalEnvelope, IterationGovernance},
};

use support::{
    run, run_json,
    workflow_fixture::{GenerationMode, WorkflowFixture},
};

#[test]
fn unsafe_sealed_evidence_is_rejected_without_persisting_a_project() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let sealed_snapshot_id = fixture.sealed_snapshot_id;
    let manifest = fixture.write_variant("unsafe-sealed.toml", move |manifest| {
        let mut sealed = manifest.development.clone();
        sealed.name = "unsafe sealed acceptance".into();
        sealed.cohorts[0].name = "unsafe sealed cohort".into();
        sealed.cohorts[0].snapshot_id = sealed_snapshot_id;
        sealed.cohorts[0].role = CohortRole::SealedAcceptance;
        sealed.cohorts[0].disclosure = DisclosureLevel::Predictions;
        sealed.cohorts[0].adaptation_eligible = true;
        manifest.sealed = Some(sealed);
    });

    let output = run(
        fixture.database_url(),
        [
            "project",
            "preview",
            manifest.to_str().expect("UTF-8 manifest path"),
        ],
    );
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(
        error.contains("sealed cohorts must be aggregate-only"),
        "{error}"
    );
    assert_eq!(
        run_json(fixture.database_url(), ["project", "list"]),
        serde_json::json!([])
    );
}

#[test]
fn overbroad_preauthorization_is_rejected_without_persisting_a_project() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest = fixture.write_variant("overbroad-authority.toml", |manifest| {
        manifest.workflow.governance = IterationGovernance::PreauthorizedBounded {
            envelope: ApprovalEnvelope {
                maximum_iterations: 1,
                maximum_additional_rows: 999,
                maximum_generation_requests: 1,
                maximum_advisor_calls: 0,
                maximum_advisor_tokens: None,
                permitted_generation_backend: "fake".into(),
                permitted_generation_model: "deterministic-v1".into(),
                permitted_training_backend: "hashing-linear".into(),
                permitted_training_configuration_fingerprints: Vec::new(),
            },
        };
    });

    let output = run(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest.to_str().expect("UTF-8 manifest path"),
        ],
    );
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(
        error.contains("preauthorization exceeds the workflow budget"),
        "{error}"
    );
    assert_eq!(
        run_json(fixture.database_url(), ["project", "list"]),
        serde_json::json!([])
    );
    assert_eq!(
        run_json(fixture.database_url(), ["workflow", "definition-list"]),
        serde_json::json!([])
    );
}

#[test]
fn workflow_cannot_initialize_after_its_pinned_cohort_role_is_retired() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let prepared = fixture.prepare();
    let preparation = &prepared["preparation"];
    let definition_id = preparation["workflow_definition_id"]
        .as_str()
        .expect("workflow definition ID");
    let development_suite_id = preparation["development_suite_id"]
        .as_str()
        .expect("development suite ID");
    let suite = run_json(
        fixture.database_url(),
        ["benchmark", "show", development_suite_id],
    );
    let cohort_id = suite["cohorts"][0]["cohort_id"]
        .as_str()
        .expect("development cohort ID");

    run_json(
        fixture.database_url(),
        [
            "cohort",
            "retire",
            cohort_id,
            "--reason",
            "benchmark was deliberately invalidated before execution",
        ],
    );
    let start = run(
        fixture.database_url(),
        ["workflow", "start", definition_id, "--initialize-only"],
    );

    assert!(!start.status.success());
    assert!(
        String::from_utf8_lossy(&start.stderr)
            .contains("current role no longer matches the immutable bundle authority"),
        "stderr: {}",
        String::from_utf8_lossy(&start.stderr)
    );
    assert_eq!(
        run_json(fixture.database_url(), ["workflow", "list"]),
        serde_json::json!([]),
        "failed authority validation must occur before a workflow run is persisted"
    );
}

#[test]
fn insufficient_development_disclosure_is_rejected_during_preparation() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest = fixture.write_variant("slices-only-development.toml", |manifest| {
        manifest.development.cohorts[0].disclosure =
            workflow_core::governance::DisclosureLevel::Slices;
    });
    let output = run(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest.to_str().expect("UTF-8 manifest path"),
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("must permit row-content disclosure and adaptive use"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        run_json(fixture.database_url(), ["project", "list"]),
        serde_json::json!([]),
        "an infeasible evidence policy must not persist a partial project"
    );
}

#[test]
fn adaptation_ineligible_development_evidence_is_rejected_during_preparation() {
    let fixture = WorkflowFixture::new(GenerationMode::Fake);
    let manifest = fixture.write_variant("adaptation-ineligible-development.toml", |manifest| {
        manifest.development.cohorts[0].adaptation_eligible = false;
    });
    let output = run(
        fixture.database_url(),
        [
            "project",
            "prepare",
            manifest.to_str().expect("UTF-8 manifest path"),
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("must permit row-content disclosure and adaptive use"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        run_json(fixture.database_url(), ["project", "list"]),
        serde_json::json!([]),
        "an infeasible evidence policy must not persist a partial project"
    );
}
