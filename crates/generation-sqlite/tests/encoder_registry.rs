use artifact_core::{ArtifactKind, ProvenanceStore};
use chrono::Utc;
use synthetic_data_sqlite::SqliteStore;
use training_core::{
    domain::{EncoderArchitecture, EncoderArtifact, EncoderMetadata, RegisteredEncoder},
    ports::EncoderRegistry,
};
use uuid::Uuid;

async fn store() -> (tempfile::TempDir, SqliteStore) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("registry.db");
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let store = SqliteStore::connect(&url).await.expect("database connects");
    (directory, store)
}

fn encoder() -> RegisteredEncoder {
    let artifact = |file_name: &str, marker: &str| EncoderArtifact {
        file_name: file_name.into(),
        path: format!("models/tiny/{file_name}"),
        size_bytes: 100,
        checksum: marker.repeat(64),
    };
    RegisteredEncoder {
        id: Uuid::new_v4(),
        name: "tiny-bert".into(),
        architecture: EncoderArchitecture::Bert,
        source_path: "models/tiny".into(),
        fingerprint: format!("sha256:{}", "d".repeat(64)),
        configuration: artifact("config.json", "a"),
        tokenizer: artifact("tokenizer.json", "b"),
        weights: artifact("model.safetensors", "c"),
        metadata: EncoderMetadata {
            vocab_size: 16,
            hidden_size: 8,
            layer_count: 1,
            attention_head_count: 2,
            intermediate_size: 16,
            maximum_position_embeddings: 16,
            type_vocab_size: 2,
            padding_token_id: 0,
            tensor_dtype: "F32".into(),
        },
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn registered_encoder_round_trips_and_has_provenance() {
    let (_directory, store) = store().await;
    let encoder = encoder();

    store
        .register_encoder(&encoder)
        .await
        .expect("register encoder");

    assert_eq!(
        store.get_encoder(encoder.id).await.expect("load encoder"),
        Some(encoder.clone())
    );
    assert_eq!(
        store.list_encoders().await.expect("list encoders"),
        vec![encoder.clone()]
    );
    let provenance = store
        .trace_provenance(ArtifactKind::BaseModel, encoder.id)
        .await
        .expect("trace")
        .expect("provenance");
    assert_eq!(
        provenance.fingerprint.as_deref(),
        Some(encoder.fingerprint.as_str())
    );
    assert!(provenance.parents.is_empty());
}
