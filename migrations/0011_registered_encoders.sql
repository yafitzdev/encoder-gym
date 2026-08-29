CREATE TABLE registered_encoders (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    architecture TEXT NOT NULL CHECK (architecture IN ('bert')),
    source_path TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    configuration_artifact_json TEXT NOT NULL,
    tokenizer_artifact_json TEXT NOT NULL,
    weights_artifact_json TEXT NOT NULL,
    metadata_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX registered_encoders_name_idx ON registered_encoders(name, created_at);
