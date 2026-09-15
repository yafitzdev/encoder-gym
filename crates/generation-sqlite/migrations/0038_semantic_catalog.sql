CREATE TABLE semantic_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    profile_key TEXT NOT NULL,
    version INTEGER NOT NULL CHECK (version > 0),
    predecessor_id TEXT REFERENCES semantic_profiles(id) ON DELETE RESTRICT,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('reusable', 'dataset')),
    scope_dataset_id TEXT REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    target_key TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    UNIQUE(profile_key, version),
    CHECK (
        (scope_kind = 'reusable' AND scope_dataset_id IS NULL) OR
        (scope_kind = 'dataset' AND scope_dataset_id IS NOT NULL)
    )
);

CREATE INDEX idx_semantic_profiles_key_version
    ON semantic_profiles(profile_key, version DESC);
CREATE INDEX idx_semantic_profiles_target
    ON semantic_profiles(target_key, scope_kind, created_at DESC);

CREATE TABLE dataset_semantic_binding_decisions (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    target_key TEXT NOT NULL,
    layer TEXT NOT NULL CHECK (layer IN ('reusable', 'dataset_override')),
    profile_id TEXT REFERENCES semantic_profiles(id) ON DELETE RESTRICT,
    profile_fingerprint TEXT,
    predecessor_id TEXT REFERENCES dataset_semantic_binding_decisions(id) ON DELETE RESTRICT,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    CHECK (
        (profile_id IS NULL AND profile_fingerprint IS NULL) OR
        (profile_id IS NOT NULL AND profile_fingerprint IS NOT NULL)
    )
);

CREATE INDEX idx_semantic_bindings_current
    ON dataset_semantic_binding_decisions(dataset_id, target_key, layer, sequence DESC);

CREATE TABLE generation_job_semantics (
    job_id TEXT PRIMARY KEY NOT NULL
        REFERENCES generation_jobs(id) ON DELETE RESTRICT,
    context_fingerprint TEXT NOT NULL,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_generation_job_semantics_context
    ON generation_job_semantics(context_fingerprint);
