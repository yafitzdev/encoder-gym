CREATE TABLE project_preparations (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    manifest_fingerprint TEXT NOT NULL UNIQUE,
    dataset_id TEXT NOT NULL REFERENCES dataset_definitions(id) ON DELETE RESTRICT,
    project_configuration_id TEXT NOT NULL REFERENCES project_configurations(id) ON DELETE RESTRICT,
    development_suite_id TEXT NOT NULL REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    sealed_suite_id TEXT REFERENCES workflow_benchmark_suites(id) ON DELETE RESTRICT,
    workflow_definition_id TEXT NOT NULL UNIQUE REFERENCES workflow_definitions(id) ON DELETE RESTRICT,
    artifact_json TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_project_preparations_created
    ON project_preparations(created_at DESC, id);
