use encoder_experiment_core::ports::EncoderTaskBackend;
use encoder_experiment_nomos::NomosBackend;

/// Explicit local process/integrity test. Ordinary workspace tests never depend on Nomos.
#[tokio::test]
#[ignore = "requires NOMOS_ENCODER_GYM_EXPERIMENT and the isolated Nomos artifact copy"]
async fn verifies_the_real_isolated_nomos_snapshot() {
    let root = std::env::var("NOMOS_ENCODER_GYM_EXPERIMENT")
        .expect("set NOMOS_ENCODER_GYM_EXPERIMENT to the isolated copy");
    let backend = NomosBackend::open(root, "python").unwrap();
    let project = backend.project_snapshot().unwrap();
    let inspection = backend.inspect(project.clone()).await.unwrap();

    assert_eq!(project.backend, backend.identity());
    assert_eq!(inspection.source_fingerprint, project.source_fingerprint);
    assert_eq!(inspection.verified_artifact_keys.len(), 8);
    assert!(
        inspection
            .verified_artifact_keys
            .iter()
            .any(|key| key.contains("fullreplay_mnrl_v1"))
    );
    assert!(
        inspection
            .verified_artifact_keys
            .iter()
            .any(|key| key.contains("fullreplay_triplet_v1"))
    );
    assert!(
        inspection
            .verified_artifact_keys
            .iter()
            .any(|key| key.contains("qwen3-0.6b-dq-onnx"))
    );
}
