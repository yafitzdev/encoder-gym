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
